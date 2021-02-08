use std::{borrow::Cow, collections::hash_map, str::FromStr};

use ahash::AHashMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName};
use smallvec::{smallvec, SmallVec};

/// A multi-map of HTTP headers.
///
/// `HeaderMap` is a "multi-map" of [`HeaderName`] to one or more [`HeaderValue`]s.
#[derive(Debug, Clone, Default)]
pub struct HeaderMap {
    pub(crate) inner: AHashMap<HeaderName, Value>,
}

#[derive(Debug, Clone)]
pub(crate) enum Value {
    One(HeaderValue),
    Multi(SmallVec<[HeaderValue; 4]>),
}

impl Value {
    fn first(&self) -> &HeaderValue {
        match self {
            Value::One(ref val) => val,
            Value::Multi(ref val) => &val[0],
        }
    }

    fn first_mut(&mut self) -> &mut HeaderValue {
        match self {
            Value::One(ref mut val) => val,
            Value::Multi(ref mut val) => &mut val[0],
        }
    }

    fn append(&mut self, val: HeaderValue) {
        match self {
            Value::One(_) => {
                let data = std::mem::replace(self, Value::Multi(smallvec![val]));
                match data {
                    Value::One(val) => self.append(val),
                    Value::Multi(_) => unreachable!(),
                }
            }
            Value::Multi(ref mut vec) => vec.push(val),
        }
    }
}

impl HeaderMap {
    /// Create an empty `HeaderMap`.
    ///
    /// The map will be created without any capacity. This function will not allocate.
    pub fn new() -> Self {
        HeaderMap::default()
    }

    /// Create an empty `HeaderMap` with the specified capacity.
    ///
    /// The returned map will allocate internal storage in order to hold about `capacity` elements
    /// without reallocating. However, this is a "best effort" as there are usage patterns that
    /// could cause additional allocations before `capacity` headers are stored in the map.
    ///
    /// More capacity than requested may be allocated.
    pub fn with_capacity(capacity: usize) -> HeaderMap {
        HeaderMap {
            inner: AHashMap::with_capacity(capacity),
        }
    }

    /// Returns the number of keys stored in the map.
    ///
    /// This number could be be less than or equal to actual headers stored in the map.
    pub fn len(&self) -> usize {
        // TODO: wat!? that's messed up
        self.inner.len()
    }

    /// Returns true if the map contains no elements.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    /// Clears the map, removing all name-value pairs. Keeps the allocated memory for reuse.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Returns the number of headers the map can hold without reallocating.
    ///
    /// This number is an approximation as certain usage patterns could cause additional allocations
    /// before the returned capacity is filled.
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserves capacity for at least `additional` more headers to be inserted in the map.
    ///
    /// The header map may reserve more space to avoid frequent reallocations. Like with
    /// `with_capacity`, this will be a "best effort" to avoid allocations until `additional` more
    /// headers are inserted. Certain usage patterns could cause additional allocations before the
    /// number is reached.
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional)
    }

    fn get_value(&self, key: impl AsHeaderName) -> Option<&Value> {
        match key.try_as_name().ok()? {
            Cow::Borrowed(name) => self.inner.get(name),
            Cow::Owned(name) => self.inner.get(&name),
        }
    }

    /// Returns a reference to the first value associated with a header name.
    ///
    /// If there are multiple values associated with the key, then the first one is returned.
    /// Use `get_all` to get all values associated with a given key. Returns `None` if there are no
    /// values associated with the key.
    pub fn get(&self, key: impl AsHeaderName) -> Option<&HeaderValue> {
        self.get_value(key).map(|val| val.first())
    }

    /// Returns a mutable reference to the first value associated a header name.
    ///
    /// If there are multiple values associated with the key, then the first one is returned.
    /// Use `get_all` to get all values associated with a given key. Returns `None` if there are no
    /// values associated with the key.
    pub fn get_mut(&mut self, key: impl AsHeaderName) -> Option<&mut HeaderValue> {
        match key.try_as_name().ok()? {
            Cow::Borrowed(name) => self.inner.get_mut(name).map(|v| v.first_mut()),
            Cow::Owned(name) => self.inner.get_mut(&name).map(|v| v.first_mut()),
        }
    }

    /// Returns an iterator of all values associated with a header name.
    ///
    /// The returned view does not incur any allocations and allows iterating the values associated
    /// with the key. Iterator will yield no items if there are no values associated with the key.
    /// Iteration order is not guaranteed to be the same as insertion order.
    pub fn get_all(&self, key: impl AsHeaderName) -> GetAll<'_> {
        GetAll::new(self.get_value(key))
    }

    /// Returns true if the map contains a value for the specified key.
    ///
    /// Invalid header names will simply return false.
    pub fn contains_key(&self, key: impl AsHeaderName) -> bool {
        match key.try_as_name() {
            Ok(Cow::Borrowed(name)) => self.inner.contains_key(name),
            Ok(Cow::Owned(name)) => self.inner.contains_key(&name),
            Err(_) => false,
        }
    }

    /// An iterator visiting all name-value pairs.
    ///
    /// The iteration order is arbitrary but consistent across platforms for the same crate version.
    /// Each key will be yielded once per associated value. So, if a key has 3 associated values, it
    /// will be yielded 3 times.
    pub fn iter(&self) -> Iter<'_> {
        Iter::new(self.inner.iter())
    }

    /// An iterator visiting all keys.
    ///
    /// The iteration order is arbitrary but consistent across platforms for the same crate version.
    /// Each key will be yielded only once even if it has multiple associated values.
    pub fn keys(&self) -> Keys<'_> {
        Keys(self.inner.keys())
    }

    /// Inserts a name-value pair into the map.
    ///
    /// If the map did have this key present, the new value is associated with the key and all
    /// previous values are removed. **Note** that only a single one of the previous values
    /// is returned. If there are multiple values that have been previously associated with the key,
    /// then the first one is returned. See `insert_mult` on `OccupiedEntry` for an API that returns
    /// all values.
    ///
    /// The key is not updated, though; this matters for types that can be `==`
    /// without being identical.
    pub fn insert(&mut self, key: HeaderName, val: HeaderValue) {
        self.inner.insert(key, Value::One(val));
    }

    /// Inserts a name-value pair into the map.
    ///
    /// If the map did have this key present, the new value is pushed to the end of the list of
    /// values currently associated with the key. The key is not updated, though; this matters for
    /// types that can be `==` without being identical.
    pub fn append(&mut self, key: HeaderName, value: HeaderValue) {
        match self.inner.entry(key) {
            hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().append(value);
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(Value::One(value));
            }
        };
    }

    /// Removes all headers for a particular header name from the map.
    // TODO: return value somehow
    pub fn remove(&mut self, key: impl AsHeaderName) {
        match key.try_as_name() {
            Ok(Cow::Borrowed(name)) => self.inner.remove(name),
            Ok(Cow::Owned(name)) => self.inner.remove(&name),
            Err(_) => None,
        };
    }
}

impl IntoIterator for HeaderMap {
    type Item = (HeaderName, HeaderValue);
    type IntoIter = IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        IntoIter::new(self.inner.into_iter())
    }
}

impl<'a> IntoIterator for &'a HeaderMap {
    type Item = (&'a HeaderName, &'a HeaderValue);
    type IntoIter = Iter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        Iter::new(self.inner.iter())
    }
}

pub trait AsHeaderName: sealed::Sealed {}

mod sealed {
    use super::*;

    pub trait Sealed {
        fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName>;
    }

    impl Sealed for HeaderName {
        fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
            Ok(Cow::Borrowed(self))
        }
    }
    impl AsHeaderName for HeaderName {}

    impl Sealed for &HeaderName {
        fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
            Ok(Cow::Borrowed(*self))
        }
    }
    impl AsHeaderName for &HeaderName {}

    impl Sealed for &str {
        fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
            HeaderName::from_str(self).map(Cow::Owned)
        }
    }
    impl AsHeaderName for &str {}

    impl Sealed for String {
        fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
            HeaderName::from_str(self).map(Cow::Owned)
        }
    }
    impl AsHeaderName for String {}

    impl Sealed for &String {
        fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
            HeaderName::from_str(self).map(Cow::Owned)
        }
    }
    impl AsHeaderName for &String {}
}

/// Iterator for all values in a `HeaderMap` with the same name.
pub struct GetAll<'a> {
    idx: usize,
    value: Option<&'a Value>,
}

impl<'a> GetAll<'a> {
    fn new(value: Option<&'a Value>) -> Self {
        Self { idx: 0, value }
    }
}

impl<'a> Iterator for GetAll<'a> {
    type Item = &'a HeaderValue;

    #[inline]
    fn next(&mut self) -> Option<&'a HeaderValue> {
        let val = self.value?;

        match val {
            Value::One(ref val) => {
                // remove value to fast-path future next calls
                self.value = None;
                Some(val)
            }
            Value::Multi(ref vec) => match vec.get(self.idx) {
                Some(val) => {
                    self.idx += 1;
                    Some(val)
                }
                None => {
                    // current index is none; remove value to fast-path future next calls
                    self.value = None;
                    None
                }
            },
        }
    }
}

pub struct Keys<'a>(hash_map::Keys<'a, HeaderName, Value>);

impl<'a> Iterator for Keys<'a> {
    type Item = &'a HeaderName;

    #[inline]
    fn next(&mut self) -> Option<&'a HeaderName> {
        self.0.next()
    }
}

pub struct Iter<'a> {
    iter: hash_map::Iter<'a, HeaderName, Value>,
    multi_inner: Option<(&'a HeaderName, &'a SmallVec<[HeaderValue; 4]>)>,
    multi_idx: usize,
}

impl<'a> Iter<'a> {
    fn new(iter: hash_map::Iter<'a, HeaderName, Value>) -> Self {
        Self {
            iter,
            multi_idx: 0,
            multi_inner: None,
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a HeaderName, &'a HeaderValue);

    #[inline]
    fn next(&mut self) -> Option<(&'a HeaderName, &'a HeaderValue)> {
        // handle in-progress multi value lists first
        if let Some((ref name, ref mut vals)) = self.multi_inner {
            match vals.get(self.multi_idx) {
                Some(val) => {
                    self.multi_idx += 1;
                    return Some((name, val));
                }
                None => {
                    // no more items in value list; reset state
                    self.multi_idx = 0;
                    self.multi_inner = None;
                }
            }
        }

        let (name, value) = self.iter.next()?;

        match value {
            Value::One(ref value) => Some((name, value)),
            Value::Multi(ref vals) => {
                // set up new multi value inner iter and recurse into it
                self.multi_inner = Some((name, vals));
                self.next()
            }
        }
    }
}

pub struct IntoIter {
    iter: hash_map::IntoIter<HeaderName, Value>,
    multi_inner: Option<(HeaderName, smallvec::IntoIter<[HeaderValue; 4]>)>,
}

impl IntoIter {
    fn new(iter: hash_map::IntoIter<HeaderName, Value>) -> Self {
        Self {
            iter,
            multi_inner: None,
        }
    }
}

impl Iterator for IntoIter {
    type Item = (HeaderName, HeaderValue);

    #[inline]
    fn next(&mut self) -> Option<(HeaderName, HeaderValue)> {
        // handle in-progress multi value iterators first
        if let Some((ref name, ref mut vals)) = self.multi_inner {
            match vals.next() {
                Some(val) => {
                    return Some((name.clone(), val));
                }
                None => {
                    // no more items in value iterator; reset state
                    self.multi_inner = None;
                }
            }
        }

        let (name, value) = self.iter.next()?;

        match value {
            Value::One(value) => Some((name, value)),
            Value::Multi(vals) => {
                // set up new multi value inner iter and recurse into it
                self.multi_inner = Some((name, vals.into_iter()));
                self.next()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use http::header;

    use super::*;

    #[test]
    fn test_new() {
        let map = HeaderMap::new();
        assert_eq!(map.len(), 0);
        assert_eq!(map.capacity(), 0);

        let map = HeaderMap::with_capacity(16);
        assert_eq!(map.len(), 0);
        assert!(map.capacity() >= 16);
    }

    #[test]
    fn test_insert() {
        let mut map = HeaderMap::new();

        map.insert(header::LOCATION, HeaderValue::from_static("/test"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_contains() {
        let mut map = HeaderMap::new();
        assert!(!map.contains_key(header::LOCATION));

        map.insert(header::LOCATION, HeaderValue::from_static("/test"));
        assert!(map.contains_key(header::LOCATION));
        assert!(map.contains_key("Location"));
        assert!(map.contains_key("Location".to_owned()));
        assert!(map.contains_key("location"));
    }

    #[test]
    fn test_entries_iter() {
        let mut map = HeaderMap::new();

        map.append(header::HOST, HeaderValue::from_static("duck.com"));
        map.append(header::COOKIE, HeaderValue::from_static("one=1"));
        map.append(header::COOKIE, HeaderValue::from_static("two=2"));

        let mut iter = map.iter();
        assert!(iter.next().is_some());
        assert!(iter.next().is_some());
        assert!(iter.next().is_some());
        assert!(iter.next().is_none());

        let pairs = map.iter().collect::<Vec<_>>();
        assert!(pairs.contains(&(&header::HOST, &HeaderValue::from_static("duck.com"))));
        assert!(pairs.contains(&(&header::COOKIE, &HeaderValue::from_static("one=1"))));
        assert!(pairs.contains(&(&header::COOKIE, &HeaderValue::from_static("two=2"))));
    }

    #[test]
    fn test_entries_into_iter() {
        let mut map = HeaderMap::new();

        map.append(header::HOST, HeaderValue::from_static("duck.com"));
        map.append(header::COOKIE, HeaderValue::from_static("one=1"));
        map.append(header::COOKIE, HeaderValue::from_static("two=2"));

        let mut iter = map.into_iter();
        assert!(iter.next().is_some());
        assert!(iter.next().is_some());
        assert!(iter.next().is_some());
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_entries_iter_and_into_iter_same_order() {
        let mut map = HeaderMap::new();

        map.append(header::HOST, HeaderValue::from_static("duck.com"));
        map.append(header::COOKIE, HeaderValue::from_static("one=1"));
        map.append(header::COOKIE, HeaderValue::from_static("two=2"));

        let mut iter = map.iter();
        let mut into_iter = map.clone().into_iter();

        assert_eq!(iter.next().map(owned_pair), into_iter.next());
        assert_eq!(iter.next().map(owned_pair), into_iter.next());
        assert_eq!(iter.next().map(owned_pair), into_iter.next());
        assert_eq!(iter.next().map(owned_pair), into_iter.next());
    }

    fn owned_pair<'a>(
        (name, val): (&'a HeaderName, &'a HeaderValue),
    ) -> (HeaderName, HeaderValue) {
        (name.clone(), val.clone())
    }
}

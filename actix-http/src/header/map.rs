//! A multi-value [`HeaderMap`], its iterators, and a helper trait for types that can be effectively
//! borrowed as, or converted to, a [HeaderValue].

use std::{borrow::Cow, collections::hash_map, mem, str::FromStr};

use ahash::AHashMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName};
use smallvec::{smallvec, SmallVec};

pub use as_header_name::AsHeaderName;

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
    fn len(&self) -> usize {
        match self {
            Value::One(_) => 1,
            Value::Multi(vals) => vals.len(),
        }
    }

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
            Value::One(_) => match mem::replace(self, Value::Multi(smallvec![val])) {
                Value::One(val) => self.append(val),
                Value::Multi(_) => unreachable!(),
            },
            Value::Multi(ref mut vals) => vals.push(val),
        }
    }
}

impl HeaderMap {
    /// Create an empty `HeaderMap`.
    ///
    /// The map will be created without any capacity; this function will not allocate.
    pub fn new() -> Self {
        HeaderMap::default()
    }

    /// Create an empty `HeaderMap` with the specified capacity.
    ///
    /// The map will be able to hold at least `capacity` elements without needing to reallocate.
    /// If `capacity` is 0, the map will be created without allocating.
    pub fn with_capacity(capacity: usize) -> Self {
        HeaderMap {
            inner: AHashMap::with_capacity(capacity),
        }
    }

    /// Create new `HeaderMap` from a `http::HeaderMap`-like drain.
    pub(crate) fn from_drain<I>(mut drain: I) -> Self
    where
        I: Iterator<Item = (Option<HeaderName>, HeaderValue)>,
    {
        let (first_name, first_value) = match drain.next() {
            None => return HeaderMap::new(),
            Some((name, val)) => {
                let name = name.expect("drained first item had no name");
                (name, val)
            }
        };

        let (lb, ub) = drain.size_hint();
        let capacity = ub.unwrap_or(lb);

        let mut map = HeaderMap::with_capacity(capacity);
        map.append(first_name.clone(), first_value);

        let (map, _) =
            drain.fold((map, first_name), |(mut map, prev_name), (name, value)| {
                let name = name.unwrap_or(prev_name.clone());
                map.append(name.clone(), value);
                (map, name)
            });

        map
    }

    /// Returns the number of values stored in the map.
    ///
    /// Also see [`len_keys`](Self::len_keys).
    pub fn len(&self) -> usize {
        self.inner
            .iter()
            .fold(0, |acc, (_, values)| acc + values.len())
    }

    /// Returns the number of _keys_ stored in the map.
    ///
    /// The number of _values_ stored will be at least this number. Also see [`Self::len`].
    pub fn len_keys(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the map contains no elements.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    /// Clears the map, removing all name-value pairs.
    ///
    /// Keeps the allocated memory for reuse.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    fn get_value(&self, key: impl AsHeaderName) -> Option<&Value> {
        match key.try_as_name().ok()? {
            Cow::Borrowed(name) => self.inner.get(name),
            Cow::Owned(name) => self.inner.get(&name),
        }
    }

    /// Returns a reference to the _first_ value associated with a header name.
    ///
    /// Even when multiple values associated with the key, the "first" one is returned but is not
    /// guaranteed to be chosen in with particular order. Use `get_all` to get all values associated
    /// with a given key. Returns `None` if there are no values associated with the key.
    pub fn get(&self, key: impl AsHeaderName) -> Option<&HeaderValue> {
        self.get_value(key).map(|val| val.first())
    }

    /// Returns a mutable reference to the _first_ value associated a header name.
    ///
    /// Even when multiple values associated with the key, the "first" one is returned but is not
    /// guaranteed to be chosen in with particular order. Use `get_all` to get all values associated
    /// with a given key. Returns `None` if there are no values associated with the key.
    pub fn get_mut(&mut self, key: impl AsHeaderName) -> Option<&mut HeaderValue> {
        match key.try_as_name().ok()? {
            Cow::Borrowed(name) => self.inner.get_mut(name).map(|v| v.first_mut()),
            Cow::Owned(name) => self.inner.get_mut(&name).map(|v| v.first_mut()),
        }
    }

    /// Returns an iterator over all values associated with a header name.
    ///
    /// The returned iterator does not incur any allocations and will yield no items if there are no
    /// values associated with the key. Iteration order is **not** guaranteed to be the same as
    /// insertion order.
    pub fn get_all(&self, key: impl AsHeaderName) -> GetAll<'_> {
        GetAll::new(self.get_value(key))
    }

    /// Returns `true` if the map contains a value for the specified key.
    ///
    /// Invalid header names will simply return false.
    pub fn contains_key(&self, key: impl AsHeaderName) -> bool {
        match key.try_as_name() {
            Ok(Cow::Borrowed(name)) => self.inner.contains_key(name),
            Ok(Cow::Owned(name)) => self.inner.contains_key(&name),
            Err(_) => false,
        }
    }

    /// Inserts a name-value pair into the map.
    ///
    /// If the map already contained this key, the new value is associated with the key and all
    /// previous values are removed and returned as a `Removed` iterator. The key is not updated;
    /// this matters for types that can be `==` without being identical.
    pub fn insert(&mut self, key: HeaderName, val: HeaderValue) -> Removed {
        Removed::new(self.inner.insert(key, Value::One(val)))
    }

    /// Inserts a name-value pair into the map.
    ///
    /// If the map already contained this key, the new value is added to the list of values
    /// currently associated with the key. The key is not updated; this matters for types that can
    /// be `==` without being identical.
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
    pub fn remove(&mut self, key: impl AsHeaderName) -> Removed {
        let value = match key.try_as_name() {
            Ok(Cow::Borrowed(name)) => self.inner.remove(name),
            Ok(Cow::Owned(name)) => self.inner.remove(&name),
            Err(_) => None,
        };

        Removed::new(value)
    }

    /// Returns the number of single-value headers the map can hold without needing to reallocate.
    ///
    /// Since this is a multi-value map, the actual capacity is much larger when considering
    /// each header name can be associated with an arbitrary number of values. The effect is that
    /// the size of `len` may be greater than `capacity` since it counts all the values.
    /// Conversely, [`len_keys`](Self::len_keys) will never be larger than capacity.
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserves capacity for at least `additional` more headers to be inserted in the map.
    ///
    /// The header map may reserve more space to avoid frequent reallocations. Additional capacity
    /// only considers single-value headers.
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional)
    }

    /// An iterator over all name-value pairs.
    ///
    /// Names will be yielded for each associated value. So, if a key has 3 associated values, it
    /// will be yielded 3 times. The iteration order should be considered arbitrary.
    pub fn iter(&self) -> Iter<'_> {
        Iter::new(self.inner.iter())
    }

    /// An iterator over all contained header names.
    ///
    /// Each name will only be yielded once even if it has multiple associated values. The iteration
    /// order should be considered arbitrary.
    pub fn keys(&self) -> Keys<'_> {
        Keys(self.inner.keys())
    }

    /// Clears the map, returning all name-value sets as an iterator.
    ///
    /// Header names will only be yielded for the first value in each set. All items that are
    /// yielded without a name and after an item with a name are associated with that same name.
    /// The first item will always contain a name.
    ///
    /// Keeps the allocated memory for reuse.
    pub fn drain(&mut self) -> Drain<'_> {
        Drain::new(self.inner.drain())
    }
}

/// Note that this implementation will clone a [HeaderName] for each value.
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

mod as_header_name {
    use super::*;

    pub trait AsHeaderName: Sealed {}

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

// TODO: add size hints to the iterators

/// Iterator for all values with the same header name.
#[derive(Debug)]
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
    fn next(&mut self) -> Option<Self::Item> {
        let val = self.value?;

        match val {
            Value::One(ref val) => {
                // remove value to fast-path future next calls
                self.value = None;
                Some(val)
            }
            Value::Multi(ref vals) => match vals.get(self.idx) {
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

/// Iterator for owned [`HeaderValue`]s with the same associated [`HeaderName`] returned from methods
/// on [`HeaderMap`] that remove or replace items.
#[derive(Debug)]
pub struct Removed {
    inner: smallvec::IntoIter<[HeaderValue; 4]>,
}

impl<'a> Removed {
    fn new(value: Option<Value>) -> Self {
        let inner = match value {
            Some(Value::One(val)) => smallvec![val].into_iter(),
            Some(Value::Multi(vals)) => vals.into_iter(),
            None => smallvec![].into_iter(),
        };

        Self { inner }
    }
}

impl Iterator for Removed {
    type Item = HeaderValue;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// Iterator over all [`HeaderName`]s in the map.
#[derive(Debug)]
pub struct Keys<'a>(hash_map::Keys<'a, HeaderName, Value>);

impl<'a> Iterator for Keys<'a> {
    type Item = &'a HeaderName;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

#[derive(Debug)]
pub struct Iter<'a> {
    inner: hash_map::Iter<'a, HeaderName, Value>,
    multi_inner: Option<(&'a HeaderName, &'a SmallVec<[HeaderValue; 4]>)>,
    multi_idx: usize,
}

impl<'a> Iter<'a> {
    fn new(iter: hash_map::Iter<'a, HeaderName, Value>) -> Self {
        Self {
            inner: iter,
            multi_idx: 0,
            multi_inner: None,
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a HeaderName, &'a HeaderValue);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
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

        let (name, value) = self.inner.next()?;

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

/// Iterator over drained name-value pairs.
///
/// Iterator items are `(Option<HeaderName>, HeaderValue)` to avoid cloning.
#[derive(Debug)]
pub struct Drain<'a> {
    inner: hash_map::Drain<'a, HeaderName, Value>,
    multi_inner: Option<(Option<HeaderName>, SmallVec<[HeaderValue; 4]>)>,
    multi_inner_idx: usize,
}

impl<'a> Drain<'a> {
    fn new(iter: hash_map::Drain<'a, HeaderName, Value>) -> Self {
        Self {
            inner: iter,
            multi_inner: None,
            multi_inner_idx: 0,
        }
    }
}

impl<'a> Iterator for Drain<'a> {
    type Item = (Option<HeaderName>, HeaderValue);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // handle in-progress multi value iterators first
        if let Some((ref mut name, ref mut vals)) = self.multi_inner {
            if vals.len() > 0 {
                // OPTIMISE: array removals
                return Some((name.take(), vals.remove(0)));
            } else {
                // no more items in value iterator; reset state
                self.multi_inner = None;
                self.multi_inner_idx = 0;
            }
        }

        let (name, mut value) = self.inner.next()?;

        match value {
            Value::One(value) => Some((Some(name), value)),
            Value::Multi(ref mut vals) => {
                // set up new multi value inner iter and recurse into it
                self.multi_inner = Some((Some(name), mem::take(vals)));
                self.next()
            }
        }
    }
}

/// Iterator over owned name-value pairs.
///
/// Implementation necessarily clones header names for each value.
#[derive(Debug)]
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
    fn next(&mut self) -> Option<Self::Item> {
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
    fn create() {
        let map = HeaderMap::new();
        assert_eq!(map.len(), 0);
        assert_eq!(map.capacity(), 0);

        let map = HeaderMap::with_capacity(16);
        assert_eq!(map.len(), 0);
        assert!(map.capacity() >= 16);
    }

    #[test]
    fn insert() {
        let mut map = HeaderMap::new();

        map.insert(header::LOCATION, HeaderValue::from_static("/test"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn contains() {
        let mut map = HeaderMap::new();
        assert!(!map.contains_key(header::LOCATION));

        map.insert(header::LOCATION, HeaderValue::from_static("/test"));
        assert!(map.contains_key(header::LOCATION));
        assert!(map.contains_key("Location"));
        assert!(map.contains_key("Location".to_owned()));
        assert!(map.contains_key("location"));
    }

    #[test]
    fn entries_iter() {
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
    fn drain_iter() {
        let mut map = HeaderMap::new();

        map.append(header::COOKIE, HeaderValue::from_static("one=1"));
        map.append(header::COOKIE, HeaderValue::from_static("two=2"));

        let mut vals = vec![];
        let mut iter = map.drain();

        let (name, val) = iter.next().unwrap();
        assert_eq!(name, Some(header::COOKIE));
        vals.push(val);

        let (name, val) = iter.next().unwrap();
        assert!(name.is_none());
        vals.push(val);

        assert!(vals.contains(&HeaderValue::from_static("one=1")));
        assert!(vals.contains(&HeaderValue::from_static("two=2")));

        assert!(iter.next().is_none());
        drop(iter);
        
        assert!(map.is_empty());
    }

    #[test]
    fn entries_into_iter() {
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
    fn iter_and_into_iter_same_order() {
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

    #[test]
    fn get_all_and_remove_same_order() {
        let mut map = HeaderMap::new();

        map.append(header::COOKIE, HeaderValue::from_static("one=1"));
        map.append(header::COOKIE, HeaderValue::from_static("two=2"));

        let mut vals = map.get_all(header::COOKIE);
        let mut removed = map.clone().remove(header::COOKIE);

        assert_eq!(vals.next(), removed.next().as_ref());
        assert_eq!(vals.next(), removed.next().as_ref());
        assert_eq!(vals.next(), removed.next().as_ref());
    }

    fn owned_pair<'a>(
        (name, val): (&'a HeaderName, &'a HeaderValue),
    ) -> (HeaderName, HeaderValue) {
        (name.clone(), val.clone())
    }
}

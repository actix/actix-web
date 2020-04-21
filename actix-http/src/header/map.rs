use std::collections::hash_map::{self, Entry};
use std::convert::TryFrom;

use either::Either;
use fxhash::FxHashMap;
use http::header::{HeaderName, HeaderValue};

/// A set of HTTP headers
///
/// `HeaderMap` is an multi-map of [`HeaderName`] to values.
///
/// [`HeaderName`]: struct.HeaderName.html
#[derive(Debug, Clone)]
pub struct HeaderMap {
    pub(crate) inner: FxHashMap<HeaderName, Value>,
}

#[derive(Debug, Clone)]
pub(crate) enum Value {
    One(HeaderValue),
    Multi(Vec<HeaderValue>),
}

impl Value {
    fn get(&self) -> &HeaderValue {
        match self {
            Value::One(ref val) => val,
            Value::Multi(ref val) => &val[0],
        }
    }

    fn get_mut(&mut self) -> &mut HeaderValue {
        match self {
            Value::One(ref mut val) => val,
            Value::Multi(ref mut val) => &mut val[0],
        }
    }

    fn append(&mut self, val: HeaderValue) {
        match self {
            Value::One(_) => {
                let data = std::mem::replace(self, Value::Multi(vec![val]));
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
    /// The map will be created without any capacity. This function will not
    /// allocate.
    pub fn new() -> Self {
        HeaderMap {
            inner: FxHashMap::default(),
        }
    }

    /// Create an empty `HeaderMap` with the specified capacity.
    ///
    /// The returned map will allocate internal storage in order to hold about
    /// `capacity` elements without reallocating. However, this is a "best
    /// effort" as there are usage patterns that could cause additional
    /// allocations before `capacity` headers are stored in the map.
    ///
    /// More capacity than requested may be allocated.
    pub fn with_capacity(capacity: usize) -> HeaderMap {
        HeaderMap {
            inner: FxHashMap::with_capacity_and_hasher(capacity, Default::default()),
        }
    }

    /// Returns the number of keys stored in the map.
    ///
    /// This number could be be less than or equal to actual headers stored in
    /// the map.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the map contains no elements.
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    /// Clears the map, removing all key-value pairs. Keeps the allocated memory
    /// for reuse.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Returns the number of headers the map can hold without reallocating.
    ///
    /// This number is an approximation as certain usage patterns could cause
    /// additional allocations before the returned capacity is filled.
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Reserves capacity for at least `additional` more headers to be inserted
    /// into the `HeaderMap`.
    ///
    /// The header map may reserve more space to avoid frequent reallocations.
    /// Like with `with_capacity`, this will be a "best effort" to avoid
    /// allocations until `additional` more headers are inserted. Certain usage
    /// patterns could cause additional allocations before the number is
    /// reached.
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional)
    }

    /// Returns a reference to the value associated with the key.
    ///
    /// If there are multiple values associated with the key, then the first one
    /// is returned. Use `get_all` to get all values associated with a given
    /// key. Returns `None` if there are no values associated with the key.
    pub fn get<N: AsName>(&self, name: N) -> Option<&HeaderValue> {
        self.get2(name).map(|v| v.get())
    }

    fn get2<N: AsName>(&self, name: N) -> Option<&Value> {
        match name.as_name() {
            Either::Left(name) => self.inner.get(name),
            Either::Right(s) => {
                if let Ok(name) = HeaderName::try_from(s) {
                    self.inner.get(&name)
                } else {
                    None
                }
            }
        }
    }

    /// Returns a view of all values associated with a key.
    ///
    /// The returned view does not incur any allocations and allows iterating
    /// the values associated with the key.  See [`GetAll`] for more details.
    /// Returns `None` if there are no values associated with the key.
    ///
    /// [`GetAll`]: struct.GetAll.html
    pub fn get_all<N: AsName>(&self, name: N) -> GetAll<'_> {
        GetAll {
            idx: 0,
            item: self.get2(name),
        }
    }

    /// Returns a mutable reference to the value associated with the key.
    ///
    /// If there are multiple values associated with the key, then the first one
    /// is returned. Use `entry` to get all values associated with a given
    /// key. Returns `None` if there are no values associated with the key.
    pub fn get_mut<N: AsName>(&mut self, name: N) -> Option<&mut HeaderValue> {
        match name.as_name() {
            Either::Left(name) => self.inner.get_mut(name).map(|v| v.get_mut()),
            Either::Right(s) => {
                if let Ok(name) = HeaderName::try_from(s) {
                    self.inner.get_mut(&name).map(|v| v.get_mut())
                } else {
                    None
                }
            }
        }
    }

    /// Returns true if the map contains a value for the specified key.
    pub fn contains_key<N: AsName>(&self, key: N) -> bool {
        match key.as_name() {
            Either::Left(name) => self.inner.contains_key(name),
            Either::Right(s) => {
                if let Ok(name) = HeaderName::try_from(s) {
                    self.inner.contains_key(&name)
                } else {
                    false
                }
            }
        }
    }

    /// An iterator visiting all key-value pairs.
    ///
    /// The iteration order is arbitrary, but consistent across platforms for
    /// the same crate version. Each key will be yielded once per associated
    /// value. So, if a key has 3 associated values, it will be yielded 3 times.
    pub fn iter(&self) -> Iter<'_> {
        Iter::new(self.inner.iter())
    }

    /// An iterator visiting all keys.
    ///
    /// The iteration order is arbitrary, but consistent across platforms for
    /// the same crate version. Each key will be yielded only once even if it
    /// has multiple associated values.
    pub fn keys(&self) -> Keys<'_> {
        Keys(self.inner.keys())
    }

    /// Inserts a key-value pair into the map.
    ///
    /// If the map did not previously have this key present, then `None` is
    /// returned.
    ///
    /// If the map did have this key present, the new value is associated with
    /// the key and all previous values are removed. **Note** that only a single
    /// one of the previous values is returned. If there are multiple values
    /// that have been previously associated with the key, then the first one is
    /// returned. See `insert_mult` on `OccupiedEntry` for an API that returns
    /// all values.
    ///
    /// The key is not updated, though; this matters for types that can be `==`
    /// without being identical.
    pub fn insert(&mut self, key: HeaderName, val: HeaderValue) {
        let _ = self.inner.insert(key, Value::One(val));
    }

    /// Inserts a key-value pair into the map.
    ///
    /// If the map did not previously have this key present, then `false` is
    /// returned.
    ///
    /// If the map did have this key present, the new value is pushed to the end
    /// of the list of values currently associated with the key. The key is not
    /// updated, though; this matters for types that can be `==` without being
    /// identical.
    pub fn append(&mut self, key: HeaderName, value: HeaderValue) {
        match self.inner.entry(key) {
            Entry::Occupied(mut entry) => entry.get_mut().append(value),
            Entry::Vacant(entry) => {
                entry.insert(Value::One(value));
            }
        }
    }

    /// Removes all headers for a particular header name from the map.
    pub fn remove<N: AsName>(&mut self, key: N) {
        match key.as_name() {
            Either::Left(name) => {
                let _ = self.inner.remove(name);
            }
            Either::Right(s) => {
                if let Ok(name) = HeaderName::try_from(s) {
                    let _ = self.inner.remove(&name);
                }
            }
        }
    }
}

#[doc(hidden)]
pub trait AsName {
    fn as_name(&self) -> Either<&HeaderName, &str>;
}

impl AsName for HeaderName {
    fn as_name(&self) -> Either<&HeaderName, &str> {
        Either::Left(self)
    }
}

impl<'a> AsName for &'a HeaderName {
    fn as_name(&self) -> Either<&HeaderName, &str> {
        Either::Left(self)
    }
}

impl<'a> AsName for &'a str {
    fn as_name(&self) -> Either<&HeaderName, &str> {
        Either::Right(self)
    }
}

impl AsName for String {
    fn as_name(&self) -> Either<&HeaderName, &str> {
        Either::Right(self.as_str())
    }
}

impl<'a> AsName for &'a String {
    fn as_name(&self) -> Either<&HeaderName, &str> {
        Either::Right(self.as_str())
    }
}

pub struct GetAll<'a> {
    idx: usize,
    item: Option<&'a Value>,
}

impl<'a> Iterator for GetAll<'a> {
    type Item = &'a HeaderValue;

    #[inline]
    fn next(&mut self) -> Option<&'a HeaderValue> {
        if let Some(ref val) = self.item {
            match val {
                Value::One(ref val) => {
                    self.item.take();
                    Some(val)
                }
                Value::Multi(ref vec) => {
                    if self.idx < vec.len() {
                        let item = Some(&vec[self.idx]);
                        self.idx += 1;
                        item
                    } else {
                        self.item.take();
                        None
                    }
                }
            }
        } else {
            None
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

impl<'a> IntoIterator for &'a HeaderMap {
    type Item = (&'a HeaderName, &'a HeaderValue);
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct Iter<'a> {
    idx: usize,
    current: Option<(&'a HeaderName, &'a Vec<HeaderValue>)>,
    iter: hash_map::Iter<'a, HeaderName, Value>,
}

impl<'a> Iter<'a> {
    fn new(iter: hash_map::Iter<'a, HeaderName, Value>) -> Self {
        Self {
            iter,
            idx: 0,
            current: None,
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a HeaderName, &'a HeaderValue);

    #[inline]
    fn next(&mut self) -> Option<(&'a HeaderName, &'a HeaderValue)> {
        if let Some(ref mut item) = self.current {
            if self.idx < item.1.len() {
                let item = (item.0, &item.1[self.idx]);
                self.idx += 1;
                return Some(item);
            } else {
                self.idx = 0;
                self.current.take();
            }
        }
        if let Some(item) = self.iter.next() {
            match item.1 {
                Value::One(ref value) => Some((item.0, value)),
                Value::Multi(ref vec) => {
                    self.current = Some((item.0, vec));
                    self.next()
                }
            }
        } else {
            None
        }
    }
}

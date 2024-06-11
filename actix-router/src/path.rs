use std::{
    borrow::Cow,
    ops::{DerefMut, Index},
};

use serde::{de, Deserialize};

use crate::{de::PathDeserializer, Resource, ResourcePath};

#[derive(Debug, Clone)]
pub(crate) enum PathItem {
    Static(Cow<'static, str>),
    Segment(u16, u16),
}

impl Default for PathItem {
    fn default() -> Self {
        Self::Static(Cow::Borrowed(""))
    }
}

/// Resource path match information.
///
/// If resource path contains variable patterns, `Path` stores them.
#[derive(Debug, Clone, Default)]
pub struct Path<T> {
    /// Full path representation.
    path: T,

    /// Number of characters in `path` that have been processed into `segments`.
    pub(crate) skip: u16,

    /// List of processed dynamic segments; name->value pairs.
    pub(crate) segments: Vec<(Cow<'static, str>, PathItem)>,
}

impl<T: ResourcePath> Path<T> {
    pub fn new(path: T) -> Path<T> {
        Path {
            path,
            skip: 0,
            segments: Vec::new(),
        }
    }

    /// Returns reference to inner path instance.
    #[inline]
    pub fn get_ref(&self) -> &T {
        &self.path
    }

    /// Returns mutable reference to inner path instance.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.path
    }

    /// Returns full path as a string.
    #[inline]
    pub fn as_str(&self) -> &str {
        self.path.path()
    }

    /// Returns unprocessed part of the path.
    ///
    /// Returns empty string if no more is to be processed.
    #[inline]
    pub fn unprocessed(&self) -> &str {
        // clamp skip to path length
        let skip = (self.skip as usize).min(self.as_str().len());
        &self.path.path()[skip..]
    }

    /// Returns unprocessed part of the path.
    #[doc(hidden)]
    #[deprecated(since = "0.6.0", note = "Use `.as_str()` or `.unprocessed()`.")]
    #[inline]
    pub fn path(&self) -> &str {
        let skip = self.skip as usize;
        let path = self.path.path();
        if skip <= path.len() {
            &path[skip..]
        } else {
            ""
        }
    }

    /// Set new path.
    #[inline]
    pub fn set(&mut self, path: T) {
        self.path = path;
        self.skip = 0;
        self.segments.clear();
    }

    /// Reset state.
    #[inline]
    pub fn reset(&mut self) {
        self.skip = 0;
        self.segments.clear();
    }

    /// Skip first `n` chars in path.
    #[inline]
    pub fn skip(&mut self, n: u16) {
        self.skip += n;
    }

    pub(crate) fn add(&mut self, name: impl Into<Cow<'static, str>>, value: PathItem) {
        match value {
            PathItem::Static(seg) => self.segments.push((name.into(), PathItem::Static(seg))),
            PathItem::Segment(begin, end) => self.segments.push((
                name.into(),
                PathItem::Segment(self.skip + begin, self.skip + end),
            )),
        }
    }

    #[doc(hidden)]
    pub fn add_static(
        &mut self,
        name: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) {
        self.segments
            .push((name.into(), PathItem::Static(value.into())));
    }

    /// Check if there are any matched patterns.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns number of interpolated segments.
    #[inline]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Get matched parameter by name without type conversion
    pub fn get(&self, name: &str) -> Option<&str> {
        for (seg_name, val) in self.segments.iter() {
            if name == seg_name {
                return match val {
                    PathItem::Static(ref s) => Some(s),
                    PathItem::Segment(s, e) => {
                        Some(&self.path.path()[(*s as usize)..(*e as usize)])
                    }
                };
            }
        }

        None
    }

    /// Returns matched parameter by name.
    ///
    /// If keyed parameter is not available empty string is used as default value.
    pub fn query(&self, key: &str) -> &str {
        self.get(key).unwrap_or_default()
    }

    /// Return iterator to items in parameter container.
    pub fn iter(&self) -> PathIter<'_, T> {
        PathIter {
            idx: 0,
            params: self,
        }
    }

    /// Deserializes matching parameters to a specified type `U`.
    ///
    /// # Errors
    ///
    /// Returns error when dynamic path segments cannot be deserialized into a `U` type.
    pub fn load<'de, U: Deserialize<'de>>(&'de self) -> Result<U, de::value::Error> {
        Deserialize::deserialize(PathDeserializer::new(self))
    }
}

#[derive(Debug)]
pub struct PathIter<'a, T> {
    idx: usize,
    params: &'a Path<T>,
}

impl<'a, T: ResourcePath> Iterator for PathIter<'a, T> {
    type Item = (&'a str, &'a str);

    #[inline]
    fn next(&mut self) -> Option<(&'a str, &'a str)> {
        if self.idx < self.params.segment_count() {
            let idx = self.idx;
            let res = match self.params.segments[idx].1 {
                PathItem::Static(ref s) => s,
                PathItem::Segment(s, e) => &self.params.path.path()[(s as usize)..(e as usize)],
            };
            self.idx += 1;
            return Some((&self.params.segments[idx].0, res));
        }
        None
    }
}

impl<'a, T: ResourcePath> Index<&'a str> for Path<T> {
    type Output = str;

    fn index(&self, name: &'a str) -> &str {
        self.get(name)
            .expect("Value for parameter is not available")
    }
}

impl<T: ResourcePath> Index<usize> for Path<T> {
    type Output = str;

    fn index(&self, idx: usize) -> &str {
        match self.segments[idx].1 {
            PathItem::Static(ref s) => s,
            PathItem::Segment(s, e) => &self.path.path()[(s as usize)..(e as usize)],
        }
    }
}

impl<T: ResourcePath> Resource for Path<T> {
    type Path = T;

    fn resource_path(&mut self) -> &mut Path<Self::Path> {
        self
    }
}

impl<T, P> Resource for T
where
    T: DerefMut<Target = Path<P>>,
    P: ResourcePath,
{
    type Path = P;

    fn resource_path(&mut self) -> &mut Path<Self::Path> {
        &mut *self
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[allow(clippy::needless_borrow)]
    #[test]
    fn deref_impls() {
        let mut foo = Path::new("/foo");
        let _ = (&mut foo).resource_path();

        let foo = RefCell::new(foo);
        let _ = foo.borrow_mut().resource_path();
    }
}

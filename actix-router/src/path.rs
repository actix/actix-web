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

    /// Set new path while preserving and remapping existing captured segment indices.
    ///
    /// The `reindex` closure maps byte indices from the previous path to byte indices in the new
    /// path.
    #[doc(hidden)]
    pub fn update_with_reindex<F>(&mut self, path: T, mut reindex: F)
    where
        F: FnMut(u16) -> u16,
    {
        self.skip = reindex(self.skip);

        for (_, item) in &mut self.segments {
            if let PathItem::Segment(start, end) = item {
                *start = reindex(*start);
                *end = reindex(*end);

                if *start > *end {
                    *start = *end;
                }
            }
        }

        self.path = path;
        let path = self.path.path();

        self.skip = clamp_to_char_boundary(path, self.skip);

        for (_, item) in &mut self.segments {
            if let PathItem::Segment(start, end) = item {
                *start = clamp_to_char_boundary(path, *start);
                *end = clamp_to_char_boundary(path, *end);

                if *start > *end {
                    *start = *end;
                }
            }
        }
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
                    PathItem::Static(ref seg) => Some(seg),
                    PathItem::Segment(start, end) => {
                        Some(&self.path.path()[(*start as usize)..(*end as usize)])
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

fn clamp_to_char_boundary(path: &str, idx: u16) -> u16 {
    let mut idx = usize::from(idx).min(path.len());

    while idx > 0 && !path.is_char_boundary(idx) {
        idx -= 1;
    }

    idx as u16
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
                PathItem::Static(ref seg) => seg,
                PathItem::Segment(start, end) => {
                    &self.params.path.path()[(start as usize)..(end as usize)]
                }
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
            PathItem::Static(ref seg) => seg,
            PathItem::Segment(start, end) => &self.path.path()[(start as usize)..(end as usize)],
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
    use crate::ResourceDef;

    #[allow(clippy::needless_borrow)]
    #[test]
    fn deref_impls() {
        let mut foo = Path::new("/foo");
        let _ = (&mut foo).resource_path();

        let foo = RefCell::new(foo);
        let _ = foo.borrow_mut().resource_path();
    }

    #[test]
    #[expect(deprecated)]
    fn path_accessors_reflect_processed_state() {
        let mut path = Path::new("/user/alice".to_owned());

        assert_eq!(path.get_ref(), "/user/alice");
        assert_eq!(path.as_str(), "/user/alice");
        assert_eq!(path.unprocessed(), "/user/alice");

        path.skip(6);
        assert_eq!(path.unprocessed(), "alice");
        assert_eq!(path.path(), "alice");

        path.get_mut().push_str("/profile");
        assert_eq!(path.as_str(), "/user/alice/profile");
        assert_eq!(path.unprocessed(), "alice/profile");
    }

    #[test]
    fn captured_static_segments_are_queryable() {
        let mut path = Path::new("/user/alice");

        path.add_static("kind", "user");
        assert!(!path.is_empty());
        assert_eq!(path.segment_count(), 1);
        assert_eq!(path.get("kind"), Some("user"));
        assert_eq!(path.query("kind"), "user");
        assert_eq!(path.query("missing"), "");
        assert_eq!(&path["kind"], "user");
        assert_eq!(&path[0], "user");
        assert_eq!(path.iter().collect::<Vec<_>>(), vec![("kind", "user")]);
        assert!(path.iter().nth(1).is_none());
    }

    #[test]
    fn set_replaces_path_state() {
        let mut path = Path::new("/user/alice".to_owned());
        path.skip(6);
        path.add_static("kind", "user");

        path.set("/new".to_owned());
        assert_eq!(path.as_str(), "/new");
        assert_eq!(path.unprocessed(), "/new");
        assert!(path.is_empty());
        assert_eq!(path.segment_count(), 0);
    }

    #[test]
    fn reset_clears_captured_state() {
        let mut path = Path::new("/new");
        path.skip(1);
        path.add_static("kind", "new");

        path.reset();
        assert!(path.is_empty());
        assert_eq!(path.unprocessed(), "/new");
    }

    #[test]
    #[expect(deprecated)]
    fn deprecated_path_clamps_oversized_skip() {
        let mut path = Path::new("/new");
        path.skip(100);
        assert_eq!(path.path(), "");
    }

    #[test]
    fn remapping_preserves_valid_utf8_boundaries() {
        let resource = ResourceDef::new("/{id}/{kind}");
        let mut path = Path::new("/abc/def".to_owned());
        assert!(resource.capture_match_info(&mut path));

        path.update_with_reindex("/é".to_owned(), |index| match index {
            1 => 3,
            4 => 2,
            5 => 1,
            index => index,
        });

        assert_eq!(path.unprocessed(), "");
        assert_eq!(path.get("id"), Some(""));
        assert_eq!(path.get("kind"), Some("é"));
    }

    #[test]
    fn path_load_deserializes_captured_values() {
        let resource = ResourceDef::new("/{id}");
        let mut path = Path::new("/123");
        assert!(resource.capture_match_info(&mut path));

        let id: u32 = path.load().unwrap();
        assert_eq!(id, 123);
    }
}

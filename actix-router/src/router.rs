use crate::{IntoPatterns, Resource, ResourceDef};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ResourceId(pub u16);

/// Resource router.
///
/// It matches a [routing resource](Resource) to an ordered list of _routes_. Each is defined by a
/// single [`ResourceDef`] and contains two types of custom data:
/// 1. The route _value_, of the generic type `T`.
/// 1. Some _context_ data, of the generic type `U`, which is only provided to the check function in
///    [`recognize_fn`](Self::recognize_fn). This parameter defaults to `()` and can be omitted if
///    not required.
pub struct Router<T, U = ()> {
    routes: Vec<(ResourceDef, T, U)>,
}

impl<T, U> Router<T, U> {
    /// Constructs new `RouterBuilder` with empty route list.
    pub fn build() -> RouterBuilder<T, U> {
        RouterBuilder { routes: Vec::new() }
    }

    /// Finds the value in the router that matches a given [routing resource](Resource).
    ///
    /// The match result, including the captured dynamic segments, in the `resource`.
    pub fn recognize<R>(&self, resource: &mut R) -> Option<(&T, ResourceId)>
    where
        R: Resource,
    {
        self.recognize_fn(resource, |_, _| true)
    }

    /// Same as [`recognize`](Self::recognize) but returns a mutable reference to the matched value.
    pub fn recognize_mut<R>(&mut self, resource: &mut R) -> Option<(&mut T, ResourceId)>
    where
        R: Resource,
    {
        self.recognize_mut_fn(resource, |_, _| true)
    }

    /// Finds the value in the router that matches a given [routing resource](Resource) and passes
    /// an additional predicate check using context data.
    ///
    /// Similar to [`recognize`](Self::recognize). However, before accepting the route as matched,
    /// the `check` closure is executed, passing the resource and each route's context data. If the
    /// closure returns true then the match result is stored into `resource` and a reference to
    /// the matched _value_ is returned.
    pub fn recognize_fn<R, F>(&self, resource: &mut R, mut check: F) -> Option<(&T, ResourceId)>
    where
        R: Resource,
        F: FnMut(&R, &U) -> bool,
    {
        for (rdef, val, ctx) in self.routes.iter() {
            if rdef.capture_match_info_fn(resource, |res| check(res, ctx)) {
                return Some((val, ResourceId(rdef.id())));
            }
        }

        None
    }

    /// Same as [`recognize_fn`](Self::recognize_fn) but returns a mutable reference to the matched
    /// value.
    pub fn recognize_mut_fn<R, F>(
        &mut self,
        resource: &mut R,
        mut check: F,
    ) -> Option<(&mut T, ResourceId)>
    where
        R: Resource,
        F: FnMut(&R, &U) -> bool,
    {
        for (rdef, val, ctx) in self.routes.iter_mut() {
            if rdef.capture_match_info_fn(resource, |res| check(res, ctx)) {
                return Some((val, ResourceId(rdef.id())));
            }
        }

        None
    }
}

/// Builder for an ordered [routing](Router) list.
pub struct RouterBuilder<T, U = ()> {
    routes: Vec<(ResourceDef, T, U)>,
}

impl<T, U> RouterBuilder<T, U> {
    /// Adds a new route to the end of the routing list.
    ///
    /// Returns mutable references to elements of the new route.
    pub fn push(
        &mut self,
        rdef: ResourceDef,
        val: T,
        ctx: U,
    ) -> (&mut ResourceDef, &mut T, &mut U) {
        self.routes.push((rdef, val, ctx));
        #[allow(clippy::map_identity)] // map is used to distribute &mut-ness to tuple elements
        self.routes
            .last_mut()
            .map(|(rdef, val, ctx)| (rdef, val, ctx))
            .unwrap()
    }

    /// Finish configuration and create router instance.
    pub fn finish(self) -> Router<T, U> {
        Router {
            routes: self.routes,
        }
    }
}

/// Convenience methods provided when context data impls [`Default`]
impl<T, U> RouterBuilder<T, U>
where
    U: Default,
{
    /// Registers resource for specified path.
    pub fn path(&mut self, path: impl IntoPatterns, val: T) -> (&mut ResourceDef, &mut T, &mut U) {
        self.push(ResourceDef::new(path), val, U::default())
    }

    /// Registers resource for specified path prefix.
    pub fn prefix(
        &mut self,
        prefix: impl IntoPatterns,
        val: T,
    ) -> (&mut ResourceDef, &mut T, &mut U) {
        self.push(ResourceDef::prefix(prefix), val, U::default())
    }

    /// Registers resource for [`ResourceDef`].
    pub fn rdef(&mut self, rdef: ResourceDef, val: T) -> (&mut ResourceDef, &mut T, &mut U) {
        self.push(rdef, val, U::default())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        path::Path,
        router::{ResourceId, Router},
    };

    #[allow(clippy::cognitive_complexity)]
    #[test]
    fn test_recognizer_1() {
        let mut router = Router::<usize>::build();
        router.path("/name", 10).0.set_id(0);
        router.path("/name/{val}", 11).0.set_id(1);
        router.path("/name/{val}/index.html", 12).0.set_id(2);
        router.path("/file/{file}.{ext}", 13).0.set_id(3);
        router.path("/v{val}/{val2}/index.html", 14).0.set_id(4);
        router.path("/v/{tail:.*}", 15).0.set_id(5);
        router.path("/test2/{test}.html", 16).0.set_id(6);
        router.path("/{test}/index.html", 17).0.set_id(7);
        let mut router = router.finish();

        let mut path = Path::new("/unknown");
        assert!(router.recognize_mut(&mut path).is_none());

        let mut path = Path::new("/name");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 10);
        assert_eq!(info, ResourceId(0));
        assert!(path.is_empty());

        let mut path = Path::new("/name/value");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 11);
        assert_eq!(info, ResourceId(1));
        assert_eq!(path.get("val").unwrap(), "value");
        assert_eq!(&path["val"], "value");

        let mut path = Path::new("/name/value2/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 12);
        assert_eq!(info, ResourceId(2));
        assert_eq!(path.get("val").unwrap(), "value2");

        let mut path = Path::new("/file/file.gz");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 13);
        assert_eq!(info, ResourceId(3));
        assert_eq!(path.get("file").unwrap(), "file");
        assert_eq!(path.get("ext").unwrap(), "gz");

        let mut path = Path::new("/v2/ttt/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 14);
        assert_eq!(info, ResourceId(4));
        assert_eq!(path.get("val").unwrap(), "2");
        assert_eq!(path.get("val2").unwrap(), "ttt");

        let mut path = Path::new("/v/blah-blah/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 15);
        assert_eq!(info, ResourceId(5));
        assert_eq!(path.get("tail").unwrap(), "blah-blah/index.html");

        let mut path = Path::new("/test2/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 16);
        assert_eq!(info, ResourceId(6));
        assert_eq!(path.get("test").unwrap(), "index");

        let mut path = Path::new("/bbb/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 17);
        assert_eq!(info, ResourceId(7));
        assert_eq!(path.get("test").unwrap(), "bbb");
    }

    #[test]
    fn test_recognizer_2() {
        let mut router = Router::<usize>::build();
        router.path("/index.json", 10);
        router.path("/{source}.json", 11);
        let mut router = router.finish();

        let mut path = Path::new("/index.json");
        let (h, _) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 10);

        let mut path = Path::new("/test.json");
        let (h, _) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 11);
    }

    #[test]
    fn test_recognizer_with_prefix() {
        let mut router = Router::<usize>::build();
        router.path("/name", 10).0.set_id(0);
        router.path("/name/{val}", 11).0.set_id(1);
        let mut router = router.finish();

        let mut path = Path::new("/name");
        path.skip(5);
        assert!(router.recognize_mut(&mut path).is_none());

        let mut path = Path::new("/test/name");
        path.skip(5);
        let (h, _) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 10);

        let mut path = Path::new("/test/name/value");
        path.skip(5);
        let (h, id) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 11);
        assert_eq!(id, ResourceId(1));
        assert_eq!(path.get("val").unwrap(), "value");
        assert_eq!(&path["val"], "value");

        // same patterns
        let mut router = Router::<usize>::build();
        router.path("/name", 10);
        router.path("/name/{val}", 11);
        let mut router = router.finish();

        // test skip beyond path length
        let mut path = Path::new("/name");
        path.skip(6);
        assert!(router.recognize_mut(&mut path).is_none());

        let mut path = Path::new("/test2/name");
        path.skip(6);
        let (h, _) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 10);

        let mut path = Path::new("/test2/name-test");
        path.skip(6);
        assert!(router.recognize_mut(&mut path).is_none());

        let mut path = Path::new("/test2/name/ttt");
        path.skip(6);
        let (h, _) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 11);
        assert_eq!(&path["val"], "ttt");
    }
}

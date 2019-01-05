use std::collections::HashMap;
use std::rc::Rc;

use crate::path::Path;
use crate::pattern::Pattern;
use crate::RequestPath;

#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum ResourceId {
    Default,
    Normal(u16),
}

/// Information about current resource
#[derive(Clone, Debug)]
pub struct ResourceInfo {
    rmap: Rc<ResourceMap>,
    resource: ResourceId,
}

#[derive(Default, Debug)]
pub(crate) struct ResourceMap {
    root: Option<Pattern>,
    named: HashMap<String, Pattern>,
    patterns: Vec<Pattern>,
}

/// Resource router.
pub struct Router<T> {
    rmap: Rc<ResourceMap>,
    named: HashMap<String, Pattern>,
    resources: Vec<T>,
}

impl<T> Router<T> {
    pub fn build() -> RouterBuilder<T> {
        RouterBuilder {
            rmap: ResourceMap::default(),
            named: HashMap::new(),
            resources: Vec::new(),
        }
    }

    pub fn recognize<U: RequestPath>(&self, path: &mut Path<U>) -> Option<(&T, ResourceInfo)> {
        if !path.path().is_empty() {
            for (idx, resource) in self.rmap.patterns.iter().enumerate() {
                if resource.match_path(path) {
                    let info = ResourceInfo {
                        rmap: self.rmap.clone(),
                        resource: ResourceId::Normal(idx as u16),
                    };
                    return Some((&self.resources[idx], info));
                }
            }
        }
        None
    }

    pub fn recognize_mut<U: RequestPath>(
        &mut self,
        path: &mut Path<U>,
    ) -> Option<(&mut T, ResourceInfo)> {
        if !path.path().is_empty() {
            for (idx, resource) in self.rmap.patterns.iter().enumerate() {
                if resource.match_path(path) {
                    let info = ResourceInfo {
                        rmap: self.rmap.clone(),
                        resource: ResourceId::Normal(idx as u16),
                    };
                    return Some((&mut self.resources[idx], info));
                }
            }
        }
        None
    }
}

impl ResourceMap {
    fn register(&mut self, pattern: Pattern) {
        self.patterns.push(pattern);
    }

    fn register_named(&mut self, name: String, pattern: Pattern) {
        self.patterns.push(pattern.clone());
        self.named.insert(name, pattern);
    }

    fn has_resource(&self, path: &str) -> bool {
        unimplemented!()
    }
}

pub struct RouterBuilder<T> {
    rmap: ResourceMap,
    named: HashMap<String, Pattern>,
    resources: Vec<T>,
}

impl<T> RouterBuilder<T> {
    pub fn path(&mut self, path: &str, resource: T) {
        self.rmap.register(Pattern::new(path));
        self.resources.push(resource);
    }

    pub fn prefix(&mut self, prefix: &str, resource: T) {
        self.rmap.register(Pattern::prefix(prefix));
        self.resources.push(resource);
    }

    pub fn finish(self) -> Router<T> {
        Router {
            rmap: Rc::new(self.rmap),
            named: self.named,
            resources: self.resources,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::path::Path;
    use crate::router::{ResourceId, Router};

    #[test]
    fn test_recognizer_1() {
        let mut router = Router::<usize>::build();
        router.path("/name", 10);
        router.path("/name/{val}", 11);
        router.path("/name/{val}/index.html", 12);
        router.path("/file/{file}.{ext}", 13);
        router.path("/v{val}/{val2}/index.html", 14);
        router.path("/v/{tail:.*}", 15);
        router.path("/test2/{test}.html", 16);
        router.path("/{test}/index.html", 17);
        let mut router = router.finish();

        let mut path = Path::new("/unknown");
        assert!(router.recognize_mut(&mut path).is_none());

        let mut path = Path::new("/name");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 10);
        assert_eq!(info.resource, ResourceId::Normal(0));
        assert!(path.is_empty());

        let mut path = Path::new("/name/value");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 11);
        assert_eq!(info.resource, ResourceId::Normal(1));
        assert_eq!(path.get("val").unwrap(), "value");
        assert_eq!(&path["val"], "value");

        let mut path = Path::new("/name/value2/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 12);
        assert_eq!(info.resource, ResourceId::Normal(2));
        assert_eq!(path.get("val").unwrap(), "value2");

        let mut path = Path::new("/file/file.gz");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 13);
        assert_eq!(info.resource, ResourceId::Normal(3));
        assert_eq!(path.get("file").unwrap(), "file");
        assert_eq!(path.get("ext").unwrap(), "gz");

        let mut path = Path::new("/vtest/ttt/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 14);
        assert_eq!(info.resource, ResourceId::Normal(4));
        assert_eq!(path.get("val").unwrap(), "test");
        assert_eq!(path.get("val2").unwrap(), "ttt");

        let mut path = Path::new("/v/blah-blah/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 15);
        assert_eq!(info.resource, ResourceId::Normal(5));
        assert_eq!(path.get("tail").unwrap(), "blah-blah/index.html");

        let mut path = Path::new("/test2/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 16);
        assert_eq!(info.resource, ResourceId::Normal(6));
        assert_eq!(path.get("test").unwrap(), "index");

        let mut path = Path::new("/bbb/index.html");
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 17);
        assert_eq!(info.resource, ResourceId::Normal(7));
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
        router.path("/name", 10);
        router.path("/name/{val}", 11);
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
        let (h, info) = router.recognize_mut(&mut path).unwrap();
        assert_eq!(*h, 11);
        assert_eq!(info.resource, ResourceId::Normal(1));
        assert_eq!(path.get("val").unwrap(), "value");
        assert_eq!(&path["val"], "value");

        // same patterns
        let mut router = Router::<usize>::build();
        router.path("/name", 10);
        router.path("/name/{val}", 11);
        let mut router = router.finish();

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

    // #[test]
    // fn test_request_resource() {
    //     let mut router = Router::<()>::default();
    //     let mut resource = Resource::new(ResourcePattern::new("/index.json"));
    //     resource.name("r1");
    //     router.register_resource(resource);
    //     let mut resource = Resource::new(ResourcePattern::new("/test.json"));
    //     resource.name("r2");
    //     router.register_resource(resource);

    //     let req = TestRequest::with_uri("/index.json").finish();
    //     let info = router.recognize(&req, &(), 0);
    //     assert_eq!(info.resource, ResourceId::Normal(0));

    //     assert_eq!(info.name(), "r1");

    //     let req = TestRequest::with_uri("/test.json").finish();
    //     let info = router.recognize(&req, &(), 0);
    //     assert_eq!(info.resource, ResourceId::Normal(1));
    //     assert_eq!(info.name(), "r2");
    // }

    // #[test]
    // fn test_has_resource() {
    //     let mut router = Router::<()>::default();
    //     let scope = Scope::new("/test").resource("/name", |_| "done");
    //     router.register_scope(scope);

    //     {
    //         let info = router.default_route_info();
    //         assert!(!info.has_resource("/test"));
    //         assert!(info.has_resource("/test/name"));
    //     }

    //     let scope = Scope::new("/test2").nested("/test10", |s| s.resource("/name", |_| "done"));
    //     router.register_scope(scope);

    //     let info = router.default_route_info();
    //     assert!(info.has_resource("/test2/test10/name"));
    // }

    // #[test]
    // fn test_url_for() {
    //     let mut router = Router::<()>::new(ResourcePattern::prefix(""));

    //     let mut resource = Resource::new(ResourcePattern::new("/tttt"));
    //     resource.name("r0");
    //     router.register_resource(resource);

    //     let scope = Scope::new("/test").resource("/name", |r| {
    //         r.name("r1");
    //     });
    //     router.register_scope(scope);

    //     let scope =
    //         Scope::new("/test2").nested("/test10", |s| s.resource("/name", |r| r.name("r2")));
    //     router.register_scope(scope);
    //     router.finish();

    //     let req = TestRequest::with_uri("/test").request();
    //     {
    //         let info = router.default_route_info();

    //         let res = info
    //             .url_for(&req, "r0", Vec::<&'static str>::new())
    //             .unwrap();
    //         assert_eq!(res.as_str(), "http://localhost:8080/tttt");

    //         let res = info
    //             .url_for(&req, "r1", Vec::<&'static str>::new())
    //             .unwrap();
    //         assert_eq!(res.as_str(), "http://localhost:8080/test/name");

    //         let res = info
    //             .url_for(&req, "r2", Vec::<&'static str>::new())
    //             .unwrap();
    //         assert_eq!(res.as_str(), "http://localhost:8080/test2/test10/name");
    //     }

    //     let req = TestRequest::with_uri("/test/name").request();
    //     let info = router.recognize(&req, &(), 0);
    //     assert_eq!(info.resource, ResourceId::Normal(1));

    //     let res = info
    //         .url_for(&req, "r0", Vec::<&'static str>::new())
    //         .unwrap();
    //     assert_eq!(res.as_str(), "http://localhost:8080/tttt");

    //     let res = info
    //         .url_for(&req, "r1", Vec::<&'static str>::new())
    //         .unwrap();
    //     assert_eq!(res.as_str(), "http://localhost:8080/test/name");

    //     let res = info
    //         .url_for(&req, "r2", Vec::<&'static str>::new())
    //         .unwrap();
    //     assert_eq!(res.as_str(), "http://localhost:8080/test2/test10/name");
    // }

    // #[test]
    // fn test_url_for_dynamic() {
    //     let mut router = Router::<()>::new(ResourcePattern::prefix(""));

    //     let mut resource = Resource::new(ResourcePattern::new("/{name}/test/index.{ext}"));
    //     resource.name("r0");
    //     router.register_resource(resource);

    //     let scope = Scope::new("/{name1}").nested("/{name2}", |s| {
    //         s.resource("/{name3}/test/index.{ext}", |r| r.name("r2"))
    //     });
    //     router.register_scope(scope);
    //     router.finish();

    //     let req = TestRequest::with_uri("/test").request();
    //     {
    //         let info = router.default_route_info();

    //         let res = info.url_for(&req, "r0", vec!["sec1", "html"]).unwrap();
    //         assert_eq!(res.as_str(), "http://localhost:8080/sec1/test/index.html");

    //         let res = info
    //             .url_for(&req, "r2", vec!["sec1", "sec2", "sec3", "html"])
    //             .unwrap();
    //         assert_eq!(
    //             res.as_str(),
    //             "http://localhost:8080/sec1/sec2/sec3/test/index.html"
    //         );
    //     }
    // }
}

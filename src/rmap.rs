use std::cell::RefCell;
use std::rc::{Rc, Weak};

use actix_router::ResourceDef;
use ahash::AHashMap;
use url::Url;

use crate::error::UrlGenerationError;
use crate::request::HttpRequest;

#[derive(Clone, Debug)]
pub struct ResourceMap {
    pattern: ResourceDef,

    /// Named resources within the tree or, for external resources,
    /// it points to isolated nodes outside the tree.
    named: AHashMap<String, Rc<ResourceMap>>,

    parent: RefCell<Weak<ResourceMap>>,

    /// Must be `None` for "edge" nodes.
    nodes: Option<Vec<Rc<ResourceMap>>>,
}

impl ResourceMap {
    /// Creates a _container_ node in the `ResourceMap` tree.
    pub fn new(root: ResourceDef) -> Self {
        ResourceMap {
            pattern: root,
            named: AHashMap::default(),
            parent: RefCell::new(Weak::new()),
            nodes: Some(Vec::new()),
        }
    }

    /// Adds a (possibly nested) resource.
    ///
    /// To add a non-prefix pattern, `nested` must be `None`.
    /// To add external resource, supply a pattern without a leading `/`.
    /// The root pattern of `nested`, if present, should match `pattern`.
    pub fn add(&mut self, pattern: &mut ResourceDef, nested: Option<Rc<ResourceMap>>) {
        pattern.set_id(self.nodes.as_ref().unwrap().len() as u16);

        if let Some(new_node) = nested {
            assert_eq!(&new_node.pattern, pattern, "`patern` and `nested` mismatch");
            self.named.extend(new_node.named.clone().into_iter());
            self.nodes.as_mut().unwrap().push(new_node);
        } else {
            let new_node = Rc::new(ResourceMap {
                pattern: pattern.clone(),
                named: AHashMap::default(),
                parent: RefCell::new(Weak::new()),
                nodes: None,
            });

            if let Some(name) = pattern.name() {
                self.named.insert(name.to_owned(), Rc::clone(&new_node));
            }

            let is_external = match pattern.pattern() {
                Some(p) => !p.is_empty() && !p.starts_with('/'),
                None => false,
            };

            // Don't add external resources to the tree
            if !is_external {
                self.nodes.as_mut().unwrap().push(new_node);
            }
        }
    }

    pub(crate) fn finish(self: &Rc<Self>) {
        for node in self.nodes.iter().flatten() {
            node.parent.replace(Rc::downgrade(self));
            ResourceMap::finish(node);
        }
    }

    /// Generate url for named resource
    ///
    /// Check [`HttpRequest::url_for`] for detailed information.
    pub fn url_for<U, I>(
        &self,
        req: &HttpRequest,
        name: &str,
        elements: U,
    ) -> Result<Url, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        let mut elements = elements.into_iter();

        let path = self
            .named
            .get(name)
            .ok_or(UrlGenerationError::ResourceNotFound)?
            .root_rmap_fn(String::with_capacity(24), |mut acc, node| {
                node.pattern
                    .resource_path_from_iter(&mut acc, &mut elements)
                    .then(|| acc)
            })
            .ok_or(UrlGenerationError::NotEnoughElements)?;

        if path.starts_with('/') {
            let conn = req.connection_info();
            Ok(Url::parse(&format!(
                "{}://{}{}",
                conn.scheme(),
                conn.host(),
                path
            ))?)
        } else {
            Ok(Url::parse(&path)?)
        }
    }

    pub fn has_resource(&self, path: &str) -> bool {
        self.find_matching_node(path).is_some()
    }

    /// Returns the name of the route that matches the given path or None if no full match
    /// is possible or the matching resource is not named.
    pub fn match_name(&self, path: &str) -> Option<&str> {
        self.find_matching_node(path)?.pattern.name()
    }

    /// Returns the full resource pattern matched against a path or None if no full match
    /// is possible.
    pub fn match_pattern(&self, path: &str) -> Option<String> {
        self.find_matching_node(path)?.root_rmap_fn(
            String::with_capacity(24),
            |mut acc, node| {
                acc.push_str(node.pattern.pattern()?);
                Some(acc)
            },
        )
    }

    fn find_matching_node(&self, path: &str) -> Option<&ResourceMap> {
        self._find_matching_node(path).flatten()
    }

    /// Returns `None` if root pattern doesn't match;
    /// `Some(None)` if root pattern matches but there is no matching child pattern.
    /// Don't search sideways when `Some(none)` is returned.
    fn _find_matching_node(&self, path: &str) -> Option<Option<&ResourceMap>> {
        let matched_len = self.pattern.find_match(path)?;
        let path = &path[matched_len..];

        Some(match &self.nodes {
            // find first sub-node to match remaining path
            Some(nodes) => nodes
                .iter()
                .filter_map(|node| node._find_matching_node(path))
                .next()
                .flatten(),

            // only terminate at edge nodes
            None => Some(self),
        })
    }

    /// Find `self`'s highest ancestor and then run `F`, providing `B`, in that rmap context.
    fn root_rmap_fn<F, B>(&self, init: B, mut f: F) -> Option<B>
    where
        F: FnMut(B, &ResourceMap) -> Option<B>,
    {
        self._root_rmap_fn(init, &mut f)
    }

    /// Run `F`, providing `B`, if `self` is top-level resource map, else recurse to parent map.
    fn _root_rmap_fn<F, B>(&self, init: B, f: &mut F) -> Option<B>
    where
        F: FnMut(B, &ResourceMap) -> Option<B>,
    {
        let data = match self.parent.borrow().upgrade() {
            Some(ref parent) => parent._root_rmap_fn(init, f)?,
            None => init,
        };

        f(data, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_matched_pattern() {
        let mut root = ResourceMap::new(ResourceDef::root_prefix(""));

        let mut user_map = ResourceMap::new(ResourceDef::root_prefix("/user/{id}"));
        user_map.add(&mut ResourceDef::new("/"), None);
        user_map.add(&mut ResourceDef::new("/profile"), None);
        user_map.add(&mut ResourceDef::new("/article/{id}"), None);
        user_map.add(&mut ResourceDef::new("/post/{post_id}"), None);
        user_map.add(
            &mut ResourceDef::new("/post/{post_id}/comment/{comment_id}"),
            None,
        );

        root.add(&mut ResourceDef::new("/info"), None);
        root.add(&mut ResourceDef::new("/v{version:[[:digit:]]{1}}"), None);
        root.add(
            &mut ResourceDef::root_prefix("/user/{id}"),
            Some(Rc::new(user_map)),
        );
        root.add(&mut ResourceDef::new("/info"), None);

        let root = Rc::new(root);
        ResourceMap::finish(&root);

        // sanity check resource map setup

        assert!(root.has_resource("/info"));
        assert!(!root.has_resource("/bar"));

        assert!(root.has_resource("/v1"));
        assert!(root.has_resource("/v2"));
        assert!(!root.has_resource("/v33"));

        assert!(!root.has_resource("/user/22"));
        assert!(root.has_resource("/user/22/"));
        assert!(root.has_resource("/user/22/profile"));

        // extract patterns from paths

        assert!(root.match_pattern("/bar").is_none());
        assert!(root.match_pattern("/v44").is_none());

        assert_eq!(root.match_pattern("/info"), Some("/info".to_owned()));
        assert_eq!(
            root.match_pattern("/v1"),
            Some("/v{version:[[:digit:]]{1}}".to_owned())
        );
        assert_eq!(
            root.match_pattern("/v2"),
            Some("/v{version:[[:digit:]]{1}}".to_owned())
        );
        assert_eq!(
            root.match_pattern("/user/22/profile"),
            Some("/user/{id}/profile".to_owned())
        );
        assert_eq!(
            root.match_pattern("/user/602CFB82-7709-4B17-ADCF-4C347B6F2203/profile"),
            Some("/user/{id}/profile".to_owned())
        );
        assert_eq!(
            root.match_pattern("/user/22/article/44"),
            Some("/user/{id}/article/{id}".to_owned())
        );
        assert_eq!(
            root.match_pattern("/user/22/post/my-post"),
            Some("/user/{id}/post/{post_id}".to_owned())
        );
        assert_eq!(
            root.match_pattern("/user/22/post/other-post/comment/42"),
            Some("/user/{id}/post/{post_id}/comment/{comment_id}".to_owned())
        );
    }

    #[test]
    fn extract_matched_name() {
        let mut root = ResourceMap::new(ResourceDef::root_prefix(""));

        let mut rdef = ResourceDef::new("/info");
        rdef.set_name("root_info");
        root.add(&mut rdef, None);

        let mut user_map = ResourceMap::new(ResourceDef::root_prefix("/user/{id}"));
        let mut rdef = ResourceDef::new("/");
        user_map.add(&mut rdef, None);

        let mut rdef = ResourceDef::new("/post/{post_id}");
        rdef.set_name("user_post");
        user_map.add(&mut rdef, None);

        root.add(
            &mut ResourceDef::root_prefix("/user/{id}"),
            Some(Rc::new(user_map)),
        );

        let root = Rc::new(root);
        ResourceMap::finish(&root);

        // sanity check resource map setup

        assert!(root.has_resource("/info"));
        assert!(!root.has_resource("/bar"));

        assert!(!root.has_resource("/user/22"));
        assert!(root.has_resource("/user/22/"));
        assert!(root.has_resource("/user/22/post/55"));

        // extract patterns from paths

        assert!(root.match_name("/bar").is_none());
        assert!(root.match_name("/v44").is_none());

        assert_eq!(root.match_name("/info"), Some("root_info"));
        assert_eq!(root.match_name("/user/22"), None);
        assert_eq!(root.match_name("/user/22/"), None);
        assert_eq!(root.match_name("/user/22/post/55"), Some("user_post"));
    }

    #[test]
    fn bug_fix_issue_1582_debug_print_exits() {
        // ref: https://github.com/actix/actix-web/issues/1582
        let mut root = ResourceMap::new(ResourceDef::root_prefix(""));

        let mut user_map = ResourceMap::new(ResourceDef::root_prefix("/user/{id}"));
        user_map.add(&mut ResourceDef::new("/"), None);
        user_map.add(&mut ResourceDef::new("/profile"), None);
        user_map.add(&mut ResourceDef::new("/article/{id}"), None);
        user_map.add(&mut ResourceDef::new("/post/{post_id}"), None);
        user_map.add(
            &mut ResourceDef::new("/post/{post_id}/comment/{comment_id}"),
            None,
        );

        root.add(
            &mut ResourceDef::root_prefix("/user/{id}"),
            Some(Rc::new(user_map)),
        );

        let root = Rc::new(root);
        ResourceMap::finish(&root);

        // check root has no parent
        assert!(root.parent.borrow().upgrade().is_none());
        // check child has parent reference
        assert!(root.nodes.as_ref().unwrap()[0]
            .parent
            .borrow()
            .upgrade()
            .is_some());
        // check child's parent root id matches root's root id
        assert!(Rc::ptr_eq(
            &root.nodes.as_ref().unwrap()[0]
                .parent
                .borrow()
                .upgrade()
                .unwrap(),
            &root
        ));

        let output = format!("{:?}", root);
        assert!(output.starts_with("ResourceMap {"));
        assert!(output.ends_with(" }"));
    }

    #[test]
    fn short_circuit() {
        let mut root = ResourceMap::new(ResourceDef::prefix(""));

        let mut user_root = ResourceDef::prefix("/user");
        let mut user_map = ResourceMap::new(user_root.clone());
        user_map.add(&mut ResourceDef::new("/u1"), None);
        user_map.add(&mut ResourceDef::new("/u2"), None);

        root.add(&mut ResourceDef::new("/user/u3"), None);
        root.add(&mut user_root, Some(Rc::new(user_map)));
        root.add(&mut ResourceDef::new("/user/u4"), None);

        let rmap = Rc::new(root);
        ResourceMap::finish(&rmap);

        assert!(rmap.has_resource("/user/u1"));
        assert!(rmap.has_resource("/user/u2"));
        assert!(rmap.has_resource("/user/u3"));
        assert!(!rmap.has_resource("/user/u4"));
    }

    #[test]
    fn url_for() {
        let mut root = ResourceMap::new(ResourceDef::prefix(""));

        let mut user_scope_rdef = ResourceDef::prefix("/user");
        let mut user_scope_map = ResourceMap::new(user_scope_rdef.clone());

        let mut user_rdef = ResourceDef::new("/{user_id}");
        let mut user_map = ResourceMap::new(user_rdef.clone());

        let mut post_rdef = ResourceDef::new("/post/{sub_id}");
        post_rdef.set_name("post");

        user_map.add(&mut post_rdef, None);
        user_scope_map.add(&mut user_rdef, Some(Rc::new(user_map)));
        root.add(&mut user_scope_rdef, Some(Rc::new(user_scope_map)));

        let rmap = Rc::new(root);
        ResourceMap::finish(&rmap);

        let mut req = crate::test::TestRequest::default();
        req.set_server_hostname("localhost:8888");
        let req = req.to_http_request();

        let url = rmap
            .url_for(&req, "post", &["u123", "foobar"])
            .unwrap()
            .to_string();
        assert_eq!(url, "http://localhost:8888/user/u123/post/foobar");

        assert!(rmap.url_for(&req, "missing", &["u123"]).is_err());
    }

    #[test]
    fn external_resource_with_no_name() {
        let mut root = ResourceMap::new(ResourceDef::prefix(""));

        let mut rdef = ResourceDef::new("https://duck.com/{query}");
        root.add(&mut rdef, None);

        let rmap = Rc::new(root);
        ResourceMap::finish(&rmap);

        assert!(!rmap.has_resource("https://duck.com/abc"));
    }

    #[test]
    fn external_resource_with_name() {
        let mut root = ResourceMap::new(ResourceDef::prefix(""));

        let mut rdef = ResourceDef::new("https://duck.com/{query}");
        rdef.set_name("duck");
        root.add(&mut rdef, None);

        let rmap = Rc::new(root);
        ResourceMap::finish(&rmap);

        assert!(!rmap.has_resource("https://duck.com/abc"));

        let mut req = crate::test::TestRequest::default();
        req.set_server_hostname("localhost:8888");
        let req = req.to_http_request();

        assert_eq!(
            rmap.url_for(&req, "duck", &["abcd"]).unwrap().to_string(),
            "https://duck.com/abcd"
        );
    }
}

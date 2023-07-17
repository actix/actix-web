use std::{
    borrow::Cow,
    cell::RefCell,
    fmt::Write as _,
    rc::{Rc, Weak},
};

use actix_router::ResourceDef;
use ahash::AHashMap;
use url::Url;

use crate::{error::UrlGenerationError, request::HttpRequest};

const AVG_PATH_LEN: usize = 24;

#[derive(Clone, Debug)]
pub struct ResourceMap {
    pattern: ResourceDef,

    /// Named resources within the tree or, for external resources, it points to isolated nodes
    /// outside the tree.
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

    /// Format resource map as tree structure (unfinished).
    #[allow(dead_code)]
    pub(crate) fn tree(&self) -> String {
        let mut buf = String::new();
        self._tree(&mut buf, 0);
        buf
    }

    pub(crate) fn _tree(&self, buf: &mut String, level: usize) {
        if let Some(children) = &self.nodes {
            for child in children {
                writeln!(
                    buf,
                    "{}{} {}",
                    "--".repeat(level),
                    child.pattern.pattern().unwrap(),
                    child
                        .pattern
                        .name()
                        .map(|name| format!("({})", name))
                        .unwrap_or_else(|| "".to_owned())
                )
                .unwrap();

                ResourceMap::_tree(child, buf, level + 1);
            }
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
            debug_assert_eq!(
                &new_node.pattern, pattern,
                "`pattern` and `nested` mismatch"
            );
            // parents absorb references to the named resources of children
            self.named.extend(new_node.named.clone());
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

            // don't add external resources to the tree
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

    /// Generate URL for named resource.
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
            .root_rmap_fn(String::with_capacity(AVG_PATH_LEN), |mut acc, node| {
                node.pattern
                    .resource_path_from_iter(&mut acc, &mut elements)
                    .then_some(acc)
            })
            .ok_or(UrlGenerationError::NotEnoughElements)?;

        let (base, path): (Cow<'_, _>, _) = if path.starts_with('/') {
            // build full URL from connection info parts and resource path
            let conn = req.connection_info();
            let base = format!("{}://{}", conn.scheme(), conn.host());
            (Cow::Owned(base), path.as_str())
        } else {
            // external resource; third slash would be the root slash in the path
            let third_slash_index = path
                .char_indices()
                .filter_map(|(i, c)| (c == '/').then_some(i))
                .nth(2)
                .unwrap_or(path.len());

            (
                Cow::Borrowed(&path[..third_slash_index]),
                &path[third_slash_index..],
            )
        };

        let mut url = Url::parse(&base)?;
        url.set_path(path);
        Ok(url)
    }

    /// Returns true if there is a resource that would match `path`.
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
            String::with_capacity(AVG_PATH_LEN),
            |mut acc, node| {
                let pattern = node.pattern.pattern()?;
                acc.push_str(pattern);
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
            .url_for(&req, "post", ["u123", "foobar"])
            .unwrap()
            .to_string();
        assert_eq!(url, "http://localhost:8888/user/u123/post/foobar");

        assert!(rmap.url_for(&req, "missing", ["u123"]).is_err());
    }

    #[test]
    fn url_for_parser() {
        let mut root = ResourceMap::new(ResourceDef::prefix(""));

        let mut rdef_1 = ResourceDef::new("/{var}");
        rdef_1.set_name("internal");

        let mut rdef_2 = ResourceDef::new("http://host.dom/{var}");
        rdef_2.set_name("external.1");

        let mut rdef_3 = ResourceDef::new("{var}");
        rdef_3.set_name("external.2");

        root.add(&mut rdef_1, None);
        root.add(&mut rdef_2, None);
        root.add(&mut rdef_3, None);
        let rmap = Rc::new(root);
        ResourceMap::finish(&rmap);

        let mut req = crate::test::TestRequest::default();
        req.set_server_hostname("localhost:8888");
        let req = req.to_http_request();

        const INPUT: &[&str] = &["a/../quick brown%20fox/%nan?query#frag"];
        const OUTPUT: &str = "/quick%20brown%20fox/%nan%3Fquery%23frag";

        let url = rmap.url_for(&req, "internal", INPUT).unwrap();
        assert_eq!(url.path(), OUTPUT);

        let url = rmap.url_for(&req, "external.1", INPUT).unwrap();
        assert_eq!(url.path(), OUTPUT);

        assert!(rmap.url_for(&req, "external.2", INPUT).is_err());
        assert!(rmap.url_for(&req, "external.2", [""]).is_err());
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
            rmap.url_for(&req, "duck", ["abcd"]).unwrap().to_string(),
            "https://duck.com/abcd"
        );
    }

    #[test]
    fn url_for_override_within_map() {
        let mut root = ResourceMap::new(ResourceDef::prefix(""));

        let mut foo_rdef = ResourceDef::prefix("/foo");
        let mut foo_map = ResourceMap::new(foo_rdef.clone());
        let mut nested_rdef = ResourceDef::new("/nested");
        nested_rdef.set_name("nested");
        foo_map.add(&mut nested_rdef, None);
        root.add(&mut foo_rdef, Some(Rc::new(foo_map)));

        let mut foo_rdef = ResourceDef::prefix("/bar");
        let mut foo_map = ResourceMap::new(foo_rdef.clone());
        let mut nested_rdef = ResourceDef::new("/nested");
        nested_rdef.set_name("nested");
        foo_map.add(&mut nested_rdef, None);
        root.add(&mut foo_rdef, Some(Rc::new(foo_map)));

        let rmap = Rc::new(root);
        ResourceMap::finish(&rmap);

        let req = crate::test::TestRequest::default().to_http_request();

        let url = rmap.url_for(&req, "nested", [""; 0]).unwrap().to_string();
        assert_eq!(url, "http://localhost:8080/bar/nested");

        assert!(rmap.url_for(&req, "missing", ["u123"]).is_err());
    }
}

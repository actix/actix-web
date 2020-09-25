use std::cell::RefCell;
use std::rc::{Rc, Weak};

use actix_router::ResourceDef;
use fxhash::FxHashMap;
use url::Url;

use crate::error::UrlGenerationError;
use crate::request::HttpRequest;

#[derive(Clone, Debug)]
pub struct ResourceMap {
    root: ResourceDef,
    parent: RefCell<Weak<ResourceMap>>,
    named: FxHashMap<String, ResourceDef>,
    patterns: Vec<(ResourceDef, Option<Rc<ResourceMap>>)>,
}

impl ResourceMap {
    pub fn new(root: ResourceDef) -> Self {
        ResourceMap {
            root,
            parent: RefCell::new(Weak::new()),
            named: FxHashMap::default(),
            patterns: Vec::new(),
        }
    }

    pub fn add(&mut self, pattern: &mut ResourceDef, nested: Option<Rc<ResourceMap>>) {
        pattern.set_id(self.patterns.len() as u16);
        self.patterns.push((pattern.clone(), nested));
        if !pattern.name().is_empty() {
            self.named
                .insert(pattern.name().to_string(), pattern.clone());
        }
    }

    pub(crate) fn finish(&self, current: Rc<ResourceMap>) {
        for (_, nested) in &self.patterns {
            if let Some(ref nested) = nested {
                *nested.parent.borrow_mut() = Rc::downgrade(&current);
                nested.finish(nested.clone());
            }
        }
    }

    /// Generate url for named resource
    ///
    /// Check [`HttpRequest::url_for()`](../struct.HttpRequest.html#method.
    /// url_for) for detailed information.
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
        let mut path = String::new();
        let mut elements = elements.into_iter();

        if self.patterns_for(name, &mut path, &mut elements)?.is_some() {
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
        } else {
            Err(UrlGenerationError::ResourceNotFound)
        }
    }

    pub fn has_resource(&self, path: &str) -> bool {
        let path = if path.is_empty() { "/" } else { path };

        for (pattern, rmap) in &self.patterns {
            if let Some(ref rmap) = rmap {
                if let Some(plen) = pattern.is_prefix_match(path) {
                    return rmap.has_resource(&path[plen..]);
                }
            } else if pattern.is_match(path) {
                return true;
            }
        }
        false
    }

    /// Returns the name of the route that matches the given path or None if no full match
    /// is possible.
    pub fn match_name(&self, path: &str) -> Option<&str> {
        let path = if path.is_empty() { "/" } else { path };

        for (pattern, rmap) in &self.patterns {
            if let Some(ref rmap) = rmap {
                if let Some(plen) = pattern.is_prefix_match(path) {
                    return rmap.match_name(&path[plen..]);
                }
            } else if pattern.is_match(path) {
                return match pattern.name() {
                    "" => None,
                    s => Some(s),
                };
            }
        }

        None
    }

    /// Returns the full resource pattern matched against a path or None if no full match
    /// is possible.
    pub fn match_pattern(&self, path: &str) -> Option<String> {
        let path = if path.is_empty() { "/" } else { path };

        // ensure a full match exists
        if !self.has_resource(path) {
            return None;
        }

        Some(self.traverse_resource_pattern(path))
    }

    /// Takes remaining path and tries to match it up against a resource definition within the
    /// current resource map recursively, returning a concatenation of all resource prefixes and
    /// patterns matched in the tree.
    ///
    /// Should only be used after checking the resource exists in the map so that partial match
    /// patterns are not returned.
    fn traverse_resource_pattern(&self, remaining: &str) -> String {
        for (pattern, rmap) in &self.patterns {
            if let Some(ref rmap) = rmap {
                if let Some(prefix_len) = pattern.is_prefix_match(remaining) {
                    let prefix = pattern.pattern().to_owned();

                    return [
                        prefix,
                        rmap.traverse_resource_pattern(&remaining[prefix_len..]),
                    ]
                    .concat();
                }
            } else if pattern.is_match(remaining) {
                return pattern.pattern().to_owned();
            }
        }

        String::new()
    }

    fn patterns_for<U, I>(
        &self,
        name: &str,
        path: &mut String,
        elements: &mut U,
    ) -> Result<Option<()>, UrlGenerationError>
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        if self.pattern_for(name, path, elements)?.is_some() {
            Ok(Some(()))
        } else {
            self.parent_pattern_for(name, path, elements)
        }
    }

    fn pattern_for<U, I>(
        &self,
        name: &str,
        path: &mut String,
        elements: &mut U,
    ) -> Result<Option<()>, UrlGenerationError>
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        if let Some(pattern) = self.named.get(name) {
            if pattern.pattern().starts_with('/') {
                self.fill_root(path, elements)?;
            }
            if pattern.resource_path(path, elements) {
                Ok(Some(()))
            } else {
                Err(UrlGenerationError::NotEnoughElements)
            }
        } else {
            for (_, rmap) in &self.patterns {
                if let Some(ref rmap) = rmap {
                    if rmap.pattern_for(name, path, elements)?.is_some() {
                        return Ok(Some(()));
                    }
                }
            }
            Ok(None)
        }
    }

    fn fill_root<U, I>(
        &self,
        path: &mut String,
        elements: &mut U,
    ) -> Result<(), UrlGenerationError>
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        if let Some(ref parent) = self.parent.borrow().upgrade() {
            parent.fill_root(path, elements)?;
        }
        if self.root.resource_path(path, elements) {
            Ok(())
        } else {
            Err(UrlGenerationError::NotEnoughElements)
        }
    }

    fn parent_pattern_for<U, I>(
        &self,
        name: &str,
        path: &mut String,
        elements: &mut U,
    ) -> Result<Option<()>, UrlGenerationError>
    where
        U: Iterator<Item = I>,
        I: AsRef<str>,
    {
        if let Some(ref parent) = self.parent.borrow().upgrade() {
            if let Some(pattern) = parent.named.get(name) {
                self.fill_root(path, elements)?;
                if pattern.resource_path(path, elements) {
                    Ok(Some(()))
                } else {
                    Err(UrlGenerationError::NotEnoughElements)
                }
            } else {
                parent.parent_pattern_for(name, path, elements)
            }
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_matched_pattern() {
        let mut root = ResourceMap::new(ResourceDef::root_prefix(""));

        let mut user_map = ResourceMap::new(ResourceDef::root_prefix(""));
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

        let root = Rc::new(root);
        root.finish(Rc::clone(&root));

        // sanity check resource map setup

        assert!(root.has_resource("/info"));
        assert!(!root.has_resource("/bar"));

        assert!(root.has_resource("/v1"));
        assert!(root.has_resource("/v2"));
        assert!(!root.has_resource("/v33"));

        assert!(root.has_resource("/user/22"));
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
        *rdef.name_mut() = "root_info".to_owned();
        root.add(&mut rdef, None);

        let mut user_map = ResourceMap::new(ResourceDef::root_prefix(""));
        let mut rdef = ResourceDef::new("/");
        user_map.add(&mut rdef, None);

        let mut rdef = ResourceDef::new("/post/{post_id}");
        *rdef.name_mut() = "user_post".to_owned();
        user_map.add(&mut rdef, None);

        root.add(
            &mut ResourceDef::root_prefix("/user/{id}"),
            Some(Rc::new(user_map)),
        );

        let root = Rc::new(root);
        root.finish(Rc::clone(&root));

        // sanity check resource map setup

        assert!(root.has_resource("/info"));
        assert!(!root.has_resource("/bar"));

        assert!(root.has_resource("/user/22"));
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

        let mut user_map = ResourceMap::new(ResourceDef::root_prefix(""));
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
        root.finish(Rc::clone(&root));

        // check root has no parent
        assert!(root.parent.borrow().upgrade().is_none());
        // check child has parent reference
        assert!(root.patterns[0].1.is_some());
        // check child's parent root id matches root's root id
        assert_eq!(
            root.patterns[0].1.as_ref().unwrap().root.id(),
            root.root.id()
        );

        let output = format!("{:?}", root);
        assert!(output.starts_with("ResourceMap {"));
        assert!(output.ends_with(" }"));
    }
}

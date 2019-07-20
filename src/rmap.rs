use std::cell::RefCell;
use std::rc::Rc;

use actix_router::ResourceDef;
use hashbrown::HashMap;
use url::Url;

use crate::error::UrlGenerationError;
use crate::request::HttpRequest;

#[derive(Clone, Debug)]
pub struct ResourceMap {
    root: ResourceDef,
    parent: RefCell<Option<Rc<ResourceMap>>>,
    named: HashMap<String, ResourceDef>,
    patterns: Vec<(ResourceDef, Option<Rc<ResourceMap>>)>,
}

impl ResourceMap {
    pub fn new(root: ResourceDef) -> Self {
        ResourceMap {
            root,
            parent: RefCell::new(None),
            named: HashMap::new(),
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
                *nested.parent.borrow_mut() = Some(current.clone());
                nested.finish(nested.clone());
            }
        }
    }
}

impl ResourceMap {
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
        if let Some(ref parent) = *self.parent.borrow() {
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
        if let Some(ref parent) = *self.parent.borrow() {
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

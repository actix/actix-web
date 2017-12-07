use std::rc::Rc;
use std::collections::HashMap;

use error::UrlGenerationError;
use resource::Resource;
use recognizer::{Params, RouteRecognizer, PatternElement};


/// Interface for application router.
pub struct Router<S>(Rc<RouteRecognizer<Resource<S>>>);

impl<S> Router<S> {
    pub(crate) fn new(prefix: &str, map: HashMap<String, Resource<S>>) -> Router<S>
    {
        let prefix = prefix.trim().trim_right_matches('/').to_owned();
        let mut resources = Vec::new();
        for (path, resource) in map {
            resources.push((path, resource.get_name(), resource))
        }

        Router(Rc::new(RouteRecognizer::new(prefix, resources)))
    }

    /// Router prefix
    #[inline]
    pub(crate) fn prefix(&self) -> &str {
        self.0.prefix()
    }

    /// Query for matched resource
    pub fn query(&self, path: &str) -> Option<(Option<Params>, &Resource<S>)> {
        self.0.recognize(path)
    }

    /// Check if application contains matching route.
    pub fn has_route(&self, path: &str) -> bool {
        self.0.recognize(path).is_some()
    }

    /// Build named resource path
    pub fn resource_path<U, I>(&self, name: &str, elements: U)
                               -> Result<String, UrlGenerationError>
        where U: IntoIterator<Item=I>,
              I: AsRef<str>,
    {
        if let Some(pattern) = self.0.get_pattern(name) {
            let mut path = String::from(self.prefix());
            path.push('/');
            let mut iter = elements.into_iter();
            for el in pattern.elements() {
                match *el {
                    PatternElement::Str(ref s) => path.push_str(s),
                    PatternElement::Var(_) => {
                        if let Some(val) = iter.next() {
                            path.push_str(val.as_ref())
                        } else {
                            return Err(UrlGenerationError::NotEnoughElements)
                        }
                    }
                }
            }
            Ok(path)
        } else {
            Err(UrlGenerationError::ResourceNotFound)
        }
    }
}

impl<S: 'static> Clone for Router<S> {
    fn clone(&self) -> Router<S> {
        Router(Rc::clone(&self.0))
    }
}

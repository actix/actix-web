use std::mem;
use std::rc::Rc;
use std::hash::{Hash, Hasher};
use std::collections::HashMap;

use regex::{Regex, RegexSet};

use error::UrlGenerationError;
use resource::Resource;
use httprequest::HttpRequest;


/// Interface for application router.
pub struct Router<S>(Rc<Inner<S>>);

struct Inner<S> {
    prefix: String,
    regset: RegexSet,
    named: HashMap<String, (Pattern, bool)>,
    patterns: Vec<Pattern>,
    resources: Vec<Resource<S>>,
}

impl<S> Router<S> {
    /// Create new router
    pub fn new(prefix: &str, map: HashMap<Pattern, Option<Resource<S>>>) -> Router<S>
    {
        let prefix = prefix.trim().trim_right_matches('/').to_owned();
        let mut named = HashMap::new();
        let mut patterns = Vec::new();
        let mut resources = Vec::new();
        let mut paths = Vec::new();

        for (pattern, resource) in map {
            if !pattern.name().is_empty() {
                let name = pattern.name().into();
                named.insert(name, (pattern.clone(), resource.is_none()));
            }

            if let Some(resource) = resource {
                paths.push(pattern.pattern().to_owned());
                patterns.push(pattern);
                resources.push(resource);
            }
        }

        Router(Rc::new(
            Inner{ prefix: prefix,
                   regset: RegexSet::new(&paths).unwrap(),
                   named: named,
                   patterns: patterns,
                   resources: resources }))
    }

    /// Router prefix
    #[inline]
    pub fn prefix(&self) -> &str {
        &self.0.prefix
    }

    /// Query for matched resource
    pub fn recognize(&self, req: &mut HttpRequest<S>) -> Option<&Resource<S>> {
        let mut idx = None;
        {
            let path = &req.path()[self.0.prefix.len()..];
            if path.is_empty() {
                if let Some(i) = self.0.regset.matches("/").into_iter().next() {
                    idx = Some(i);
                }
            } else if let Some(i) = self.0.regset.matches(path).into_iter().next() {
                idx = Some(i);
            }
        }

        if let Some(idx) = idx {
            let path: &str = unsafe{ mem::transmute(&req.path()[self.0.prefix.len()..]) };
            self.0.patterns[idx].update_match_info(path, req);
            return Some(&self.0.resources[idx])
        } else {
            None
        }
    }

    /// Check if application contains matching route.
    pub fn has_route(&self, path: &str) -> bool {
        let p = &path[self.0.prefix.len()..];
        if p.is_empty() {
            if self.0.regset.matches("/").into_iter().next().is_some() {
                return true
            }
        } else if self.0.regset.matches(p).into_iter().next().is_some() {
            return true
        }
        false
    }

    /// Build named resource path.
    ///
    /// Check [`HttpRequest::url_for()`](../struct.HttpRequest.html#method.url_for)
    /// for detailed information.
    pub fn resource_path<U, I>(&self, name: &str, elements: U)
                               -> Result<String, UrlGenerationError>
        where U: IntoIterator<Item=I>,
              I: AsRef<str>,
    {
        if let Some(pattern) = self.0.named.get(name) {
            if pattern.1 {
                pattern.0.path(None, elements)
            } else {
                pattern.0.path(Some(&self.0.prefix), elements)
            }
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

#[derive(Debug, Clone, PartialEq)]
enum PatternElement {
    Str(String),
    Var(String),
}

#[derive(Clone)]
pub struct Pattern {
    re: Regex,
    name: String,
    pattern: String,
    names: Vec<String>,
    elements: Vec<PatternElement>,
}

impl Pattern {
    /// Parse path pattern and create new `Pattern` instance.
    ///
    /// Panics if path pattern is wrong.
    pub fn new(name: &str, path: &str) -> Self {
        let (pattern, elements) = Pattern::parse(path);

        let re = match Regex::new(&pattern) {
            Ok(re) => re,
            Err(err) => panic!("Wrong path pattern: \"{}\" {}", path, err)
        };
        let names = re.capture_names()
            .filter_map(|name| name.map(|name| name.to_owned()))
            .collect();

        Pattern {
            re: re,
            name: name.into(),
            pattern: pattern,
            names: names,
            elements: elements,
        }
    }

    /// Returns name of the pattern
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns path of the pattern
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Extract pattern parameters from the text
    pub(crate) fn update_match_info<S>(&self, text: &str, req: &mut HttpRequest<S>) {
        if !self.names.is_empty() {
            if let Some(captures) = self.re.captures(text) {
                let mut idx = 0;
                for capture in captures.iter() {
                    if let Some(ref m) = capture {
                        if idx != 0 {
                            req.match_info_mut().add(&self.names[idx-1], m.as_str());
                        }
                        idx += 1;
                    }
                }
            };
        }
    }

    /// Build pattern path.
    pub fn path<U, I>(&self, prefix: Option<&str>, elements: U)
                      -> Result<String, UrlGenerationError>
        where U: IntoIterator<Item=I>,
              I: AsRef<str>,
    {
        let mut iter = elements.into_iter();
        let mut path = if let Some(prefix) = prefix {
            let mut path = String::from(prefix);
            path.push('/');
            path
        } else {
            String::new()
        };
        for el in &self.elements {
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
    }

    fn parse(pattern: &str) -> (String, Vec<PatternElement>) {
        const DEFAULT_PATTERN: &str = "[^/]+";

        let mut re = String::from("^/");
        let mut el = String::new();
        let mut in_param = false;
        let mut in_param_pattern = false;
        let mut param_name = String::new();
        let mut param_pattern = String::from(DEFAULT_PATTERN);
        let mut elems = Vec::new();

        for (index, ch) in pattern.chars().enumerate() {
            // All routes must have a leading slash so its optional to have one
            if index == 0 && ch == '/' {
                continue;
            }

            if in_param {
                // In parameter segment: `{....}`
                if ch == '}' {
                    elems.push(PatternElement::Var(param_name.clone()));
                    re.push_str(&format!(r"(?P<{}>{})", &param_name, &param_pattern));

                    param_name.clear();
                    param_pattern = String::from(DEFAULT_PATTERN);

                    in_param_pattern = false;
                    in_param = false;
                } else if ch == ':' {
                    // The parameter name has been determined; custom pattern land
                    in_param_pattern = true;
                    param_pattern.clear();
                } else if in_param_pattern {
                    // Ignore leading whitespace for pattern
                    if !(ch == ' ' && param_pattern.is_empty()) {
                        param_pattern.push(ch);
                    }
                } else {
                    param_name.push(ch);
                }
            } else if ch == '{' {
                in_param = true;
                elems.push(PatternElement::Str(el.clone()));
                el.clear();
            } else {
                re.push(ch);
                el.push(ch);
            }
        }

        re.push('$');
        (re, elems)
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Pattern) -> bool {
        self.pattern == other.pattern
    }
}

impl Eq for Pattern {}

impl Hash for Pattern {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pattern.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use regex::Regex;
    use super::*;
    use http::{Uri, Version, Method};
    use http::header::HeaderMap;
    use std::str::FromStr;
    use payload::Payload;

    #[test]
    fn test_recognizer() {
        let mut routes = HashMap::new();
        routes.insert(Pattern::new("", "/name"), Some(Resource::default()));
        routes.insert(Pattern::new("", "/name/{val}"), Some(Resource::default()));
        routes.insert(Pattern::new("", "/name/{val}/index.html"), Some(Resource::default()));
        routes.insert(Pattern::new("", "/v{val}/{val2}/index.html"), Some(Resource::default()));
        routes.insert(Pattern::new("", "/v/{tail:.*}"), Some(Resource::default()));
        routes.insert(Pattern::new("", "{test}/index.html"), Some(Resource::default()));
        let rec = Router::new("", routes);

        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/name").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(rec.recognize(&mut req).is_some());
        assert!(req.match_info().is_empty());

        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/name/value").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(rec.recognize(&mut req).is_some());
        assert_eq!(req.match_info().get("val").unwrap(), "value");
        assert_eq!(&req.match_info()["val"], "value");

        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/name/value2/index.html").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(rec.recognize(&mut req).is_some());
        assert_eq!(req.match_info().get("val").unwrap(), "value2");

        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/vtest/ttt/index.html").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(rec.recognize(&mut req).is_some());
        assert_eq!(req.match_info().get("val").unwrap(), "test");
        assert_eq!(req.match_info().get("val2").unwrap(), "ttt");

        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/v/blah-blah/index.html").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(rec.recognize(&mut req).is_some());
        assert_eq!(req.match_info().get("tail").unwrap(), "blah-blah/index.html");

        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/bbb/index.html").unwrap(),
            Version::HTTP_11, HeaderMap::new(), Payload::empty());
        assert!(rec.recognize(&mut req).is_some());
        assert_eq!(req.match_info().get("test").unwrap(), "bbb");
    }

    fn assert_parse(pattern: &str, expected_re: &str) -> Regex {
        let (re_str, _) = Pattern::parse(pattern);
        assert_eq!(&*re_str, expected_re);
        Regex::new(&re_str).unwrap()
    }

    #[test]
    fn test_parse_static() {
        let re = assert_parse("/", r"^/$");
        assert!(re.is_match("/"));
        assert!(!re.is_match("/a"));

        let re = assert_parse("/name", r"^/name$");
        assert!(re.is_match("/name"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name/"));
        assert!(!re.is_match("/name~"));

        let re = assert_parse("/name/", r"^/name/$");
        assert!(re.is_match("/name/"));
        assert!(!re.is_match("/name"));
        assert!(!re.is_match("/name/gs"));

        let re = assert_parse("/user/profile", r"^/user/profile$");
        assert!(re.is_match("/user/profile"));
        assert!(!re.is_match("/user/profile/profile"));
    }

    #[test]
    fn test_parse_param() {
        let re = assert_parse("/user/{id}", r"^/user/(?P<id>[^/]+)$");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let captures = re.captures("/user/profile").unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "profile");
        assert_eq!(captures.name("id").unwrap().as_str(), "profile");

        let captures = re.captures("/user/1245125").unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "1245125");
        assert_eq!(captures.name("id").unwrap().as_str(), "1245125");

        let re = assert_parse(
            "/v{version}/resource/{id}",
            r"^/v(?P<version>[^/]+)/resource/(?P<id>[^/]+)$",
        );
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let captures = re.captures("/v151/resource/adahg32").unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "151");
        assert_eq!(captures.name("version").unwrap().as_str(), "151");
        assert_eq!(captures.name("id").unwrap().as_str(), "adahg32");
    }
}

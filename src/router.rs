use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::mem;
use std::rc::Rc;

use regex::{escape, Regex};

use error::UrlGenerationError;
use httprequest::HttpRequest;
use param::ParamItem;
use resource::ResourceHandler;
use server::ServerSettings;

/// Interface for application router.
pub struct Router(Rc<Inner>);

struct Inner {
    prefix: String,
    prefix_len: usize,
    named: HashMap<String, (Resource, bool)>,
    patterns: Vec<Resource>,
    srv: ServerSettings,
}

impl Router {
    /// Create new router
    pub fn new<S>(
        prefix: &str, settings: ServerSettings,
        map: Vec<(Resource, Option<ResourceHandler<S>>)>,
    ) -> (Router, Vec<ResourceHandler<S>>) {
        let prefix = prefix.trim().trim_right_matches('/').to_owned();
        let mut named = HashMap::new();
        let mut patterns = Vec::new();
        let mut resources = Vec::new();

        for (pattern, resource) in map {
            if !pattern.name().is_empty() {
                let name = pattern.name().into();
                named.insert(name, (pattern.clone(), resource.is_none()));
            }

            if let Some(resource) = resource {
                patterns.push(pattern);
                resources.push(resource);
            }
        }

        let prefix_len = prefix.len();
        (
            Router(Rc::new(Inner {
                prefix,
                prefix_len,
                named,
                patterns,
                srv: settings,
            })),
            resources,
        )
    }

    /// Router prefix
    #[inline]
    pub fn prefix(&self) -> &str {
        &self.0.prefix
    }

    /// Server settings
    #[inline]
    pub fn server_settings(&self) -> &ServerSettings {
        &self.0.srv
    }

    pub(crate) fn get_resource(&self, idx: usize) -> &Resource {
        &self.0.patterns[idx]
    }

    /// Query for matched resource
    pub fn recognize<S>(&self, req: &mut HttpRequest<S>) -> Option<usize> {
        if self.0.prefix_len > req.path().len() {
            return None;
        }
        for (idx, pattern) in self.0.patterns.iter().enumerate() {
            if pattern.match_with_params(req, self.0.prefix_len, true) {
                let url = req.url().clone();
                req.match_info_mut().set_url(url);
                req.set_resource(idx);
                req.set_prefix_len(self.0.prefix_len as u16);
                return Some(idx);
            }
        }
        None
    }

    /// Check if application contains matching route.
    ///
    /// This method does not take `prefix` into account.
    /// For example if prefix is `/test` and router contains route `/name`,
    /// following path would be recognizable `/test/name` but `has_route()` call
    /// would return `false`.
    pub fn has_route(&self, path: &str) -> bool {
        let path = if path.is_empty() { "/" } else { path };

        for pattern in &self.0.patterns {
            if pattern.is_match(path) {
                return true;
            }
        }
        false
    }

    /// Build named resource path.
    ///
    /// Check [`HttpRequest::url_for()`](../struct.HttpRequest.html#method.
    /// url_for) for detailed information.
    pub fn resource_path<U, I>(
        &self, name: &str, elements: U,
    ) -> Result<String, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        if let Some(pattern) = self.0.named.get(name) {
            pattern.0.resource_path(self, elements)
        } else {
            Err(UrlGenerationError::ResourceNotFound)
        }
    }
}

impl Clone for Router {
    fn clone(&self) -> Router {
        Router(Rc::clone(&self.0))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PatternElement {
    Str(String),
    Var(String),
}

#[derive(Clone, Debug)]
enum PatternType {
    Static(String),
    Prefix(String),
    Dynamic(Regex, Vec<String>, usize),
}

#[derive(Debug, Copy, Clone, PartialEq)]
/// Resource type
pub enum ResourceType {
    /// Normal resource
    Normal,
    /// Resource for application default handler
    Default,
    /// External resource
    External,
    /// Unknown resource type
    Unset,
}

/// Resource type describes an entry in resources table
#[derive(Clone)]
pub struct Resource {
    tp: PatternType,
    rtp: ResourceType,
    name: String,
    pattern: String,
    elements: Vec<PatternElement>,
}

impl Resource {
    /// Parse path pattern and create new `Resource` instance.
    ///
    /// Panics if path pattern is wrong.
    pub fn new(name: &str, path: &str) -> Self {
        Resource::with_prefix(name, path, "/", false)
    }

    /// Parse path pattern and create new `Resource` instance.
    ///
    /// Use `prefix` type instead of `static`.
    ///
    /// Panics if path regex pattern is wrong.
    pub fn prefix(name: &str, path: &str) -> Self {
        Resource::with_prefix(name, path, "/", true)
    }

    /// Construct external resource
    ///
    /// Panics if path pattern is wrong.
    pub fn external(name: &str, path: &str) -> Self {
        let mut resource = Resource::with_prefix(name, path, "/", false);
        resource.rtp = ResourceType::External;
        resource
    }

    /// Unset resource type
    pub(crate) fn unset() -> Resource {
        Resource {
            tp: PatternType::Static("".to_owned()),
            rtp: ResourceType::Unset,
            name: "".to_owned(),
            pattern: "".to_owned(),
            elements: Vec::new(),
        }
    }

    /// Parse path pattern and create new `Resource` instance with custom prefix
    pub fn with_prefix(name: &str, path: &str, prefix: &str, for_prefix: bool) -> Self {
        let (pattern, elements, is_dynamic, len) =
            Resource::parse(path, prefix, for_prefix);

        let tp = if is_dynamic {
            let re = match Regex::new(&pattern) {
                Ok(re) => re,
                Err(err) => panic!("Wrong path pattern: \"{}\" {}", path, err),
            };
            let names = re
                .capture_names()
                .filter_map(|name| name.map(|name| name.to_owned()))
                .collect();
            PatternType::Dynamic(re, names, len)
        } else if for_prefix {
            PatternType::Prefix(pattern.clone())
        } else {
            PatternType::Static(pattern.clone())
        };

        Resource {
            tp,
            elements,
            name: name.into(),
            rtp: ResourceType::Normal,
            pattern: path.to_owned(),
        }
    }

    /// Name of the resource
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Resource type
    pub fn rtype(&self) -> ResourceType {
        self.rtp
    }

    /// Path pattern of the resource
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Is this path a match against this resource?
    pub fn is_match(&self, path: &str) -> bool {
        match self.tp {
            PatternType::Static(ref s) => s == path,
            PatternType::Dynamic(ref re, _, _) => re.is_match(path),
            PatternType::Prefix(ref s) => path.starts_with(s),
        }
    }

    /// Are the given path and parameters a match against this resource?
    pub fn match_with_params<S>(
        &self, req: &mut HttpRequest<S>, plen: usize, insert: bool,
    ) -> bool {
        let mut segments: [ParamItem; 24] = unsafe { mem::uninitialized() };

        let (names, segments_len) = {
            let path = &req.path()[plen..];
            if insert {
                if path.is_empty() {
                    "/"
                } else {
                    path
                }
            } else {
                path
            };

            match self.tp {
                PatternType::Static(ref s) => return s == path,
                PatternType::Dynamic(ref re, ref names, _) => {
                    if let Some(captures) = re.captures(path) {
                        let mut idx = 0;
                        let mut passed = false;
                        for capture in captures.iter() {
                            if let Some(ref m) = capture {
                                if !passed {
                                    passed = true;
                                    continue;
                                }
                                segments[idx] = ParamItem::UrlSegment(
                                    (plen + m.start()) as u16,
                                    (plen + m.end()) as u16,
                                );
                                idx += 1;
                            }
                        }
                        (names, idx)
                    } else {
                        return false;
                    }
                }
                PatternType::Prefix(ref s) => return path.starts_with(s),
            }
        };

        let len = req.path().len();
        let params = req.match_info_mut();
        params.set_tail(len as u16);
        for idx in 0..segments_len {
            let name = unsafe { &*(names[idx].as_str() as *const _) };
            params.add(name, segments[idx]);
        }
        true
    }

    /// Is the given path a prefix match and do the parameters match against this resource?
    pub fn match_prefix_with_params<S>(
        &self, req: &mut HttpRequest<S>, plen: usize,
    ) -> Option<usize> {
        let mut segments: [ParamItem; 24] = unsafe { mem::uninitialized() };

        let (names, segments_len, tail_len) = {
            let path = &req.path()[plen..];
            let path = if path.is_empty() { "/" } else { path };

            match self.tp {
                PatternType::Static(ref s) => if s == path {
                    return Some(s.len());
                } else {
                    return None;
                },
                PatternType::Dynamic(ref re, ref names, len) => {
                    if let Some(captures) = re.captures(path) {
                        let mut idx = 0;
                        let mut pos = 0;
                        let mut passed = false;
                        for capture in captures.iter() {
                            if let Some(ref m) = capture {
                                if !passed {
                                    passed = true;
                                    continue;
                                }

                                segments[idx] = ParamItem::UrlSegment(
                                    (plen + m.start()) as u16,
                                    (plen + m.end()) as u16,
                                );
                                idx += 1;
                                pos = m.end();
                            }
                        }
                        (names, idx, pos + len)
                    } else {
                        return None;
                    }
                }
                PatternType::Prefix(ref s) => {
                    return if path == s {
                        Some(s.len())
                    } else if path.starts_with(s)
                        && (s.ends_with('/')
                            || path.split_at(s.len()).1.starts_with('/'))
                    {
                        if s.ends_with('/') {
                            Some(s.len() - 1)
                        } else {
                            Some(s.len())
                        }
                    } else {
                        None
                    }
                }
            }
        };

        let params = req.match_info_mut();
        params.set_tail(tail_len as u16);
        for idx in 0..segments_len {
            let name = unsafe { &*(names[idx].as_str() as *const _) };
            params.add(name, segments[idx]);
        }
        Some(tail_len)
    }

    /// Build resource path.
    pub fn resource_path<U, I>(
        &self, router: &Router, elements: U,
    ) -> Result<String, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        let mut path = match self.tp {
            PatternType::Prefix(ref p) => p.to_owned(),
            PatternType::Static(ref p) => p.to_owned(),
            PatternType::Dynamic(..) => {
                let mut path = String::new();
                let mut iter = elements.into_iter();
                for el in &self.elements {
                    match *el {
                        PatternElement::Str(ref s) => path.push_str(s),
                        PatternElement::Var(_) => {
                            if let Some(val) = iter.next() {
                                path.push_str(val.as_ref())
                            } else {
                                return Err(UrlGenerationError::NotEnoughElements);
                            }
                        }
                    }
                }
                path
            }
        };

        if self.rtp != ResourceType::External {
            let prefix = router.prefix();
            if prefix.ends_with('/') {
                if path.starts_with('/') {
                    path.insert_str(0, &prefix[..prefix.len() - 1]);
                } else {
                    path.insert_str(0, prefix);
                }
            } else {
                if !path.starts_with('/') {
                    path.insert(0, '/');
                }
                path.insert_str(0, prefix);
            }
        }
        Ok(path)
    }

    fn parse(
        pattern: &str, prefix: &str, for_prefix: bool,
    ) -> (String, Vec<PatternElement>, bool, usize) {
        const DEFAULT_PATTERN: &str = "[^/]+";

        let mut re1 = String::from("^") + prefix;
        let mut re2 = String::from(prefix);
        let mut el = String::new();
        let mut in_param = false;
        let mut in_param_pattern = false;
        let mut param_name = String::new();
        let mut param_pattern = String::from(DEFAULT_PATTERN);
        let mut is_dynamic = false;
        let mut elems = Vec::new();
        let mut len = 0;

        for (index, ch) in pattern.chars().enumerate() {
            // All routes must have a leading slash so its optional to have one
            if index == 0 && ch == '/' {
                continue;
            }

            if in_param {
                // In parameter segment: `{....}`
                if ch == '}' {
                    elems.push(PatternElement::Var(param_name.clone()));
                    re1.push_str(&format!(r"(?P<{}>{})", &param_name, &param_pattern));

                    param_name.clear();
                    param_pattern = String::from(DEFAULT_PATTERN);

                    len = 0;
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
                is_dynamic = true;
                elems.push(PatternElement::Str(el.clone()));
                el.clear();
            } else {
                re1.push_str(escape(&ch.to_string()).as_str());
                re2.push(ch);
                el.push(ch);
                len += 1;
            }
        }

        if !el.is_empty() {
            elems.push(PatternElement::Str(el.clone()));
        }

        let re = if is_dynamic {
            if !for_prefix {
                re1.push('$');
            }
            re1
        } else {
            re2
        };
        (re, elems, is_dynamic, len)
    }
}

impl PartialEq for Resource {
    fn eq(&self, other: &Resource) -> bool {
        self.pattern == other.pattern
    }
}

impl Eq for Resource {}

impl Hash for Resource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pattern.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test::TestRequest;

    #[test]
    fn test_recognizer10() {
        let routes = vec![
            (Resource::new("", "/name"), Some(ResourceHandler::default())),
            (
                Resource::new("", "/name/{val}"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("", "/name/{val}/index.html"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("", "/file/{file}.{ext}"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("", "/v{val}/{val2}/index.html"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("", "/v/{tail:.*}"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("", "/test2/{test}.html"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("", "{test}/index.html"),
                Some(ResourceHandler::default()),
            ),
        ];
        let (rec, _) = Router::new::<()>("", ServerSettings::default(), routes);

        let mut req = TestRequest::with_uri("/name").finish();
        assert_eq!(rec.recognize(&mut req), Some(0));
        assert!(req.match_info().is_empty());

        let mut req = TestRequest::with_uri("/name/value").finish();
        assert_eq!(rec.recognize(&mut req), Some(1));
        assert_eq!(req.match_info().get("val").unwrap(), "value");
        assert_eq!(&req.match_info()["val"], "value");

        let mut req = TestRequest::with_uri("/name/value2/index.html").finish();
        assert_eq!(rec.recognize(&mut req), Some(2));
        assert_eq!(req.match_info().get("val").unwrap(), "value2");

        let mut req = TestRequest::with_uri("/file/file.gz").finish();
        assert_eq!(rec.recognize(&mut req), Some(3));
        assert_eq!(req.match_info().get("file").unwrap(), "file");
        assert_eq!(req.match_info().get("ext").unwrap(), "gz");

        let mut req = TestRequest::with_uri("/vtest/ttt/index.html").finish();
        assert_eq!(rec.recognize(&mut req), Some(4));
        assert_eq!(req.match_info().get("val").unwrap(), "test");
        assert_eq!(req.match_info().get("val2").unwrap(), "ttt");

        let mut req = TestRequest::with_uri("/v/blah-blah/index.html").finish();
        assert_eq!(rec.recognize(&mut req), Some(5));
        assert_eq!(
            req.match_info().get("tail").unwrap(),
            "blah-blah/index.html"
        );

        let mut req = TestRequest::with_uri("/test2/index.html").finish();
        assert_eq!(rec.recognize(&mut req), Some(6));
        assert_eq!(req.match_info().get("test").unwrap(), "index");

        let mut req = TestRequest::with_uri("/bbb/index.html").finish();
        assert_eq!(rec.recognize(&mut req), Some(7));
        assert_eq!(req.match_info().get("test").unwrap(), "bbb");
    }

    #[test]
    fn test_recognizer_2() {
        let routes = vec![
            (
                Resource::new("", "/index.json"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("", "/{source}.json"),
                Some(ResourceHandler::default()),
            ),
        ];
        let (rec, _) = Router::new::<()>("", ServerSettings::default(), routes);

        let mut req = TestRequest::with_uri("/index.json").finish();
        assert_eq!(rec.recognize(&mut req), Some(0));

        let mut req = TestRequest::with_uri("/test.json").finish();
        assert_eq!(rec.recognize(&mut req), Some(1));
    }

    #[test]
    fn test_recognizer_with_prefix() {
        let routes = vec![
            (Resource::new("", "/name"), Some(ResourceHandler::default())),
            (
                Resource::new("", "/name/{val}"),
                Some(ResourceHandler::default()),
            ),
        ];
        let (rec, _) = Router::new::<()>("/test", ServerSettings::default(), routes);

        let mut req = TestRequest::with_uri("/name").finish();
        assert!(rec.recognize(&mut req).is_none());

        let mut req = TestRequest::with_uri("/test/name").finish();
        assert_eq!(rec.recognize(&mut req), Some(0));

        let mut req = TestRequest::with_uri("/test/name/value").finish();
        assert_eq!(rec.recognize(&mut req), Some(1));
        assert_eq!(req.match_info().get("val").unwrap(), "value");
        assert_eq!(&req.match_info()["val"], "value");

        // same patterns
        let routes = vec![
            (Resource::new("", "/name"), Some(ResourceHandler::default())),
            (
                Resource::new("", "/name/{val}"),
                Some(ResourceHandler::default()),
            ),
        ];
        let (rec, _) = Router::new::<()>("/test2", ServerSettings::default(), routes);

        let mut req = TestRequest::with_uri("/name").finish();
        assert!(rec.recognize(&mut req).is_none());
        let mut req = TestRequest::with_uri("/test2/name").finish();
        assert_eq!(rec.recognize(&mut req), Some(0));
        let mut req = TestRequest::with_uri("/test2/name-test").finish();
        assert!(rec.recognize(&mut req).is_none());
        let mut req = TestRequest::with_uri("/test2/name/ttt").finish();
        assert_eq!(rec.recognize(&mut req), Some(1));
        assert_eq!(&req.match_info()["val"], "ttt");
    }

    #[test]
    fn test_parse_static() {
        let re = Resource::new("test", "/");
        assert!(re.is_match("/"));
        assert!(!re.is_match("/a"));

        let re = Resource::new("test", "/name");
        assert!(re.is_match("/name"));
        assert!(!re.is_match("/name1"));
        assert!(!re.is_match("/name/"));
        assert!(!re.is_match("/name~"));

        let re = Resource::new("test", "/name/");
        assert!(re.is_match("/name/"));
        assert!(!re.is_match("/name"));
        assert!(!re.is_match("/name/gs"));

        let re = Resource::new("test", "/user/profile");
        assert!(re.is_match("/user/profile"));
        assert!(!re.is_match("/user/profile/profile"));
    }

    #[test]
    fn test_parse_param() {
        let re = Resource::new("test", "/user/{id}");
        assert!(re.is_match("/user/profile"));
        assert!(re.is_match("/user/2345"));
        assert!(!re.is_match("/user/2345/"));
        assert!(!re.is_match("/user/2345/sdg"));

        let mut req = TestRequest::with_uri("/user/profile").finish();
        let url = req.url().clone();
        req.match_info_mut().set_url(url);
        assert!(re.match_with_params(&mut req, 0, true));
        assert_eq!(req.match_info().get("id").unwrap(), "profile");

        let mut req = TestRequest::with_uri("/user/1245125").finish();
        let url = req.url().clone();
        req.match_info_mut().set_url(url);
        assert!(re.match_with_params(&mut req, 0, true));
        assert_eq!(req.match_info().get("id").unwrap(), "1245125");

        let re = Resource::new("test", "/v{version}/resource/{id}");
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let mut req = TestRequest::with_uri("/v151/resource/adahg32").finish();
        let url = req.url().clone();
        req.match_info_mut().set_url(url);
        assert!(re.match_with_params(&mut req, 0, true));
        assert_eq!(req.match_info().get("version").unwrap(), "151");
        assert_eq!(req.match_info().get("id").unwrap(), "adahg32");
    }

    #[test]
    fn test_resource_prefix() {
        let re = Resource::prefix("test", "/name");
        assert!(re.is_match("/name"));
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/test/test"));
        assert!(re.is_match("/name1"));
        assert!(re.is_match("/name~"));

        let re = Resource::prefix("test", "/name/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));
    }

    #[test]
    fn test_reousrce_prefix_dynamic() {
        let re = Resource::prefix("test", "/{name}/");
        assert!(re.is_match("/name/"));
        assert!(re.is_match("/name/gs"));
        assert!(!re.is_match("/name"));

        let mut req = TestRequest::with_uri("/test2/").finish();
        let url = req.url().clone();
        req.match_info_mut().set_url(url);
        assert!(re.match_with_params(&mut req, 0, true));
        assert_eq!(&req.match_info()["name"], "test2");

        let mut req =
            TestRequest::with_uri("/test2/subpath1/subpath2/index.html").finish();
        let url = req.url().clone();
        req.match_info_mut().set_url(url);
        assert!(re.match_with_params(&mut req, 0, true));
        assert_eq!(&req.match_info()["name"], "test2");
    }

    #[test]
    fn test_request_resource() {
        let routes = vec![
            (
                Resource::new("r1", "/index.json"),
                Some(ResourceHandler::default()),
            ),
            (
                Resource::new("r2", "/test.json"),
                Some(ResourceHandler::default()),
            ),
        ];
        let (router, _) = Router::new::<()>("", ServerSettings::default(), routes);

        let mut req =
            TestRequest::with_uri("/index.json").finish_with_router(router.clone());
        assert_eq!(router.recognize(&mut req), Some(0));
        let resource = req.resource();
        assert_eq!(resource.name(), "r1");

        let mut req =
            TestRequest::with_uri("/test.json").finish_with_router(router.clone());
        assert_eq!(router.recognize(&mut req), Some(1));
        let resource = req.resource();
        assert_eq!(resource.name(), "r2");
    }
}

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use regex::{escape, Regex};
use smallvec::SmallVec;
use url::Url;

use error::UrlGenerationError;
use httprequest::HttpRequest;
use param::{ParamItem, Params};
use resource::ResourceHandler;
use server::Request;

#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum RouterResource {
    Notset,
    Normal(u16),
}

/// Interface for application router.
pub struct Router(Rc<Inner>);

#[derive(Clone)]
pub struct RouteInfo {
    router: Rc<Inner>,
    resource: RouterResource,
    prefix: u16,
    params: Params,
}

impl RouteInfo {
    /// This method returns reference to matched `Resource` object.
    #[inline]
    pub fn resource(&self) -> Option<&Resource> {
        if let RouterResource::Normal(idx) = self.resource {
            Some(&self.router.patterns[idx as usize])
        } else {
            None
        }
    }

    /// Get a reference to the Params object.
    ///
    /// Params is a container for url parameters.
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment.
    #[inline]
    pub fn match_info(&self) -> &Params {
        &self.params
    }

    #[doc(hidden)]
    #[inline]
    pub fn prefix_len(&self) -> u16 {
        self.prefix
    }

    #[inline]
    pub(crate) fn merge(&self, mut params: Params) -> RouteInfo {
        let mut p = self.params.clone();
        p.set_tail(params.tail);
        for item in &params.segments {
            p.add(item.0.clone(), item.1.clone());
        }

        RouteInfo {
            params: p,
            router: self.router.clone(),
            resource: self.resource,
            prefix: self.prefix,
        }
    }

    /// Generate url for named resource
    ///
    /// Check [`HttpRequest::url_for()`](../struct.HttpRequest.html#method.
    /// url_for) for detailed information.
    pub fn url_for<U, I>(
        &self, req: &Request, name: &str, elements: U,
    ) -> Result<Url, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        if let Some(pattern) = self.router.named.get(name) {
            let path = pattern.0.resource_path(elements, &self.router.prefix)?;
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

    /// Check if application contains matching route.
    ///
    /// This method does not take `prefix` into account.
    /// For example if prefix is `/test` and router contains route `/name`,
    /// following path would be recognizable `/test/name` but `has_route()` call
    /// would return `false`.
    pub fn has_route(&self, path: &str) -> bool {
        let path = if path.is_empty() { "/" } else { path };

        for pattern in &self.router.patterns {
            if pattern.is_match(path) {
                return true;
            }
        }
        false
    }
}

struct Inner {
    prefix: String,
    prefix_len: usize,
    named: HashMap<String, (Resource, bool)>,
    patterns: Vec<Resource>,
}

impl Router {
    /// Create new router
    pub fn new<S>(
        prefix: &str, map: Vec<(Resource, Option<ResourceHandler<S>>)>,
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
            })),
            resources,
        )
    }

    /// Router prefix
    #[inline]
    pub fn prefix(&self) -> &str {
        &self.0.prefix
    }

    pub(crate) fn get_resource(&self, idx: usize) -> &Resource {
        &self.0.patterns[idx]
    }

    pub(crate) fn route_info(&self, req: &Request, prefix: u16) -> RouteInfo {
        let mut params = Params::with_url(req.url());
        params.set_tail(prefix);

        RouteInfo {
            params,
            router: self.0.clone(),
            resource: RouterResource::Notset,
            prefix: 0,
        }
    }

    pub(crate) fn route_info_params(&self, params: Params, prefix: u16) -> RouteInfo {
        RouteInfo {
            params,
            prefix,
            router: self.0.clone(),
            resource: RouterResource::Notset,
        }
    }

    pub(crate) fn default_route_info(&self, prefix: u16) -> RouteInfo {
        RouteInfo {
            prefix,
            router: self.0.clone(),
            resource: RouterResource::Notset,
            params: Params::new(),
        }
    }

    /// Query for matched resource
    pub fn recognize(&self, req: &Request) -> Option<(usize, RouteInfo)> {
        if self.0.prefix_len > req.path().len() {
            return None;
        }
        for (idx, pattern) in self.0.patterns.iter().enumerate() {
            if let Some(params) = pattern.match_with_params(req, self.0.prefix_len, true)
            {
                return Some((
                    idx,
                    RouteInfo {
                        params,
                        router: self.0.clone(),
                        resource: RouterResource::Normal(idx as u16),
                        prefix: self.0.prefix_len as u16,
                    },
                ));
            }
        }
        None
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
    Dynamic(Regex, Vec<Rc<String>>, usize),
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
#[derive(Clone, Debug)]
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

    /// Parse path pattern and create new `Resource` instance with custom prefix
    pub fn with_prefix(name: &str, path: &str, prefix: &str, for_prefix: bool) -> Self {
        let (pattern, elements, is_dynamic, len) =
            Resource::parse(path, prefix, for_prefix);

        let tp = if is_dynamic {
            let re = match Regex::new(&pattern) {
                Ok(re) => re,
                Err(err) => panic!("Wrong path pattern: \"{}\" {}", path, err),
            };
            // actix creates one router per thread
            let names = re
                .capture_names()
                .filter_map(|name| name.map(|name| Rc::new(name.to_owned())))
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
    pub fn match_with_params(
        &self, req: &Request, plen: usize, insert: bool,
    ) -> Option<Params> {
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
            PatternType::Static(ref s) => if s != path {
                None
            } else {
                Some(Params::with_url(req.url()))
            },
            PatternType::Dynamic(ref re, ref names, _) => {
                if let Some(captures) = re.captures(path) {
                    let mut params = Params::with_url(req.url());
                    let mut idx = 0;
                    let mut passed = false;
                    for capture in captures.iter() {
                        if let Some(ref m) = capture {
                            if !passed {
                                passed = true;
                                continue;
                            }
                            params.add(
                                names[idx].clone(),
                                ParamItem::UrlSegment(
                                    (plen + m.start()) as u16,
                                    (plen + m.end()) as u16,
                                ),
                            );
                            idx += 1;
                        }
                    }
                    params.set_tail(req.path().len() as u16);
                    Some(params)
                } else {
                    None
                }
            }
            PatternType::Prefix(ref s) => if !path.starts_with(s) {
                None
            } else {
                Some(Params::with_url(req.url()))
            },
        }
    }

    /// Is the given path a prefix match and do the parameters match against this resource?
    pub fn match_prefix_with_params(
        &self, req: &Request, plen: usize,
    ) -> Option<Params> {
        let path = &req.path()[plen..];
        let path = if path.is_empty() { "/" } else { path };

        match self.tp {
            PatternType::Static(ref s) => if s == path {
                Some(Params::with_url(req.url()))
            } else {
                None
            },
            PatternType::Dynamic(ref re, ref names, len) => {
                if let Some(captures) = re.captures(path) {
                    let mut params = Params::with_url(req.url());
                    let mut pos = 0;
                    let mut passed = false;
                    let mut idx = 0;
                    for capture in captures.iter() {
                        if let Some(ref m) = capture {
                            if !passed {
                                passed = true;
                                continue;
                            }

                            params.add(
                                names[idx].clone(),
                                ParamItem::UrlSegment(
                                    (plen + m.start()) as u16,
                                    (plen + m.end()) as u16,
                                ),
                            );
                            idx += 1;
                            pos = m.end();
                        }
                    }
                    params.set_tail((plen + pos + len) as u16);
                    Some(params)
                } else {
                    None
                }
            }
            PatternType::Prefix(ref s) => {
                let len = if path == s {
                    s.len()
                } else if path.starts_with(s)
                    && (s.ends_with('/') || path.split_at(s.len()).1.starts_with('/'))
                {
                    if s.ends_with('/') {
                        s.len() - 1
                    } else {
                        s.len()
                    }
                } else {
                    return None;
                };
                let mut params = Params::with_url(req.url());
                params.set_tail((plen + len) as u16);
                Some(params)
            }
        }
    }

    /// Build resource path.
    pub fn resource_path<U, I>(
        &self, elements: U, prefix: &str,
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
        let (rec, _) = Router::new::<()>("", routes);

        let req = TestRequest::with_uri("/name").finish();
        assert_eq!(rec.recognize(&req).unwrap().0, 0);
        assert!(req.match_info().is_empty());

        let req = TestRequest::with_uri("/name/value").finish();
        let info = rec.recognize(&req).unwrap().1;
        let req = req.with_route_info(info);
        assert_eq!(req.match_info().get("val").unwrap(), "value");
        assert_eq!(&req.match_info()["val"], "value");

        let req = TestRequest::with_uri("/name/value2/index.html").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 2);
        let req = req.with_route_info(info.1);
        assert_eq!(req.match_info().get("val").unwrap(), "value2");

        let req = TestRequest::with_uri("/file/file.gz").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 3);
        let req = req.with_route_info(info.1);
        assert_eq!(req.match_info().get("file").unwrap(), "file");
        assert_eq!(req.match_info().get("ext").unwrap(), "gz");

        let req = TestRequest::with_uri("/vtest/ttt/index.html").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 4);
        let req = req.with_route_info(info.1);
        assert_eq!(req.match_info().get("val").unwrap(), "test");
        assert_eq!(req.match_info().get("val2").unwrap(), "ttt");

        let req = TestRequest::with_uri("/v/blah-blah/index.html").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 5);
        let req = req.with_route_info(info.1);
        assert_eq!(
            req.match_info().get("tail").unwrap(),
            "blah-blah/index.html"
        );

        let req = TestRequest::with_uri("/test2/index.html").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 6);
        let req = req.with_route_info(info.1);
        assert_eq!(req.match_info().get("test").unwrap(), "index");

        let req = TestRequest::with_uri("/bbb/index.html").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 7);
        let req = req.with_route_info(info.1);
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
        let (rec, _) = Router::new::<()>("", routes);

        let req = TestRequest::with_uri("/index.json").finish();
        assert_eq!(rec.recognize(&req).unwrap().0, 0);

        let req = TestRequest::with_uri("/test.json").finish();
        assert_eq!(rec.recognize(&req).unwrap().0, 1);
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
        let (rec, _) = Router::new::<()>("/test", routes);

        let req = TestRequest::with_uri("/name").finish();
        assert!(rec.recognize(&req).is_none());

        let req = TestRequest::with_uri("/test/name").finish();
        assert_eq!(rec.recognize(&req).unwrap().0, 0);

        let req = TestRequest::with_uri("/test/name/value").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 1);
        let req = req.with_route_info(info.1);
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
        let (rec, _) = Router::new::<()>("/test2", routes);

        let req = TestRequest::with_uri("/name").finish();
        assert!(rec.recognize(&req).is_none());
        let req = TestRequest::with_uri("/test2/name").finish();
        assert_eq!(rec.recognize(&req).unwrap().0, 0);
        let req = TestRequest::with_uri("/test2/name-test").finish();
        assert!(rec.recognize(&req).is_none());
        let req = TestRequest::with_uri("/test2/name/ttt").finish();
        let info = rec.recognize(&req).unwrap();
        assert_eq!(info.0, 1);
        let req = req.with_route_info(info.1);
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

        let req = TestRequest::with_uri("/user/profile").finish();
        let info = re.match_with_params(&req, 0, true).unwrap();
        assert_eq!(info.get("id").unwrap(), "profile");

        let req = TestRequest::with_uri("/user/1245125").finish();
        let info = re.match_with_params(&req, 0, true).unwrap();
        assert_eq!(info.get("id").unwrap(), "1245125");

        let re = Resource::new("test", "/v{version}/resource/{id}");
        assert!(re.is_match("/v1/resource/320120"));
        assert!(!re.is_match("/v/resource/1"));
        assert!(!re.is_match("/resource"));

        let req = TestRequest::with_uri("/v151/resource/adahg32").finish();
        let info = re.match_with_params(&req, 0, true).unwrap();
        assert_eq!(info.get("version").unwrap(), "151");
        assert_eq!(info.get("id").unwrap(), "adahg32");
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

        let req = TestRequest::with_uri("/test2/").finish();
        let info = re.match_with_params(&req, 0, true).unwrap();
        assert_eq!(&info["name"], "test2");
        assert_eq!(&info[0], "test2");

        let req = TestRequest::with_uri("/test2/subpath1/subpath2/index.html").finish();
        let info = re.match_with_params(&req, 0, true).unwrap();
        assert_eq!(&info["name"], "test2");
        assert_eq!(&info[0], "test2");
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
        let (router, _) = Router::new::<()>("", routes);

        let req = TestRequest::with_uri("/index.json").finish();
        assert_eq!(router.recognize(&req).unwrap().0, 0);
        let info = router.recognize(&req).unwrap().1;
        let resource = info.resource().unwrap();
        assert_eq!(resource.name(), "r1");

        let req = TestRequest::with_uri("/test.json").finish();
        assert_eq!(router.recognize(&req).unwrap().0, 1);
        let info = router.recognize(&req).unwrap().1;
        let resource = info.resource().unwrap();
        assert_eq!(resource.name(), "r2");
    }
}

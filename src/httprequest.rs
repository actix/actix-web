//! HTTP Request message related code.
use std::cell::{Ref, RefMut};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::ops::Deref;
use std::rc::Rc;
use std::{fmt, str};

use cookie::Cookie;
use futures_cpupool::CpuPool;
use http::{header, HeaderMap, Method, StatusCode, Uri, Version};
use url::{form_urlencoded, Url};

use body::Body;
use error::{CookieParseError, UrlGenerationError};
use extensions::Extensions;
use handler::FromRequest;
use httpmessage::HttpMessage;
use httpresponse::{HttpResponse, HttpResponseBuilder};
use info::ConnectionInfo;
use param::Params;
use payload::Payload;
use router::ResourceInfo;
use server::Request;

struct Query(HashMap<String, String>);
struct Cookies(Vec<Cookie<'static>>);

/// An HTTP Request
pub struct HttpRequest<S = ()> {
    req: Option<Request>,
    state: Rc<S>,
    resource: ResourceInfo,
}

impl<S> HttpMessage for HttpRequest<S> {
    type Stream = Payload;

    #[inline]
    fn headers(&self) -> &HeaderMap {
        self.request().headers()
    }

    #[inline]
    fn payload(&self) -> Payload {
        if let Some(payload) = self.request().inner.payload.borrow_mut().take() {
            payload
        } else {
            Payload::empty()
        }
    }
}

impl<S> Deref for HttpRequest<S> {
    type Target = Request;

    fn deref(&self) -> &Request {
        self.request()
    }
}

impl<S> HttpRequest<S> {
    #[inline]
    pub(crate) fn new(
        req: Request, state: Rc<S>, resource: ResourceInfo,
    ) -> HttpRequest<S> {
        HttpRequest {
            state,
            resource,
            req: Some(req),
        }
    }

    #[inline]
    /// Construct new http request with state.
    pub(crate) fn with_state<NS>(&self, state: Rc<NS>) -> HttpRequest<NS> {
        HttpRequest {
            state,
            req: self.req.as_ref().map(|r| r.clone()),
            resource: self.resource.clone(),
        }
    }

    /// Construct new http request with empty state.
    pub fn drop_state(&self) -> HttpRequest {
        HttpRequest {
            state: Rc::new(()),
            req: self.req.as_ref().map(|r| r.clone()),
            resource: self.resource.clone(),
        }
    }

    #[inline]
    /// Construct new http request with new RouteInfo.
    pub(crate) fn with_route_info(&self, mut resource: ResourceInfo) -> HttpRequest<S> {
        resource.merge(&self.resource);

        HttpRequest {
            resource,
            req: self.req.as_ref().map(|r| r.clone()),
            state: self.state.clone(),
        }
    }

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        &self.state
    }

    #[inline]
    /// Server request
    pub fn request(&self) -> &Request {
        self.req.as_ref().unwrap()
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.request().extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.request().extensions_mut()
    }

    /// Default `CpuPool`
    #[inline]
    #[doc(hidden)]
    pub fn cpu_pool(&self) -> &CpuPool {
        self.request().server_settings().cpu_pool()
    }

    #[inline]
    /// Create http response
    pub fn response(&self, status: StatusCode, body: Body) -> HttpResponse {
        self.request().server_settings().get_response(status, body)
    }

    #[inline]
    /// Create http response builder
    pub fn build_response(&self, status: StatusCode) -> HttpResponseBuilder {
        self.request()
            .server_settings()
            .get_response_builder(status)
    }

    /// Read the Request Uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        self.request().inner.url.uri()
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.request().inner.method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.request().inner.version
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.request().inner.url.path()
    }

    /// Get *ConnectionInfo* for the correct request.
    #[inline]
    pub fn connection_info(&self) -> Ref<ConnectionInfo> {
        self.request().connection_info()
    }

    /// Generate url for named resource
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::{App, HttpRequest, HttpResponse, http};
    /// #
    /// fn index(req: HttpRequest) -> HttpResponse {
    ///     let url = req.url_for("foo", &["1", "2", "3"]); // <- generate url for "foo" resource
    ///     HttpResponse::Ok().into()
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .resource("/test/{one}/{two}/{three}", |r| {
    ///              r.name("foo");  // <- set resource name, then it could be used in `url_for`
    ///              r.method(http::Method::GET).f(|_| HttpResponse::Ok());
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn url_for<U, I>(
        &self, name: &str, elements: U,
    ) -> Result<Url, UrlGenerationError>
    where
        U: IntoIterator<Item = I>,
        I: AsRef<str>,
    {
        self.resource.url_for(&self, name, elements)
    }

    /// Generate url for named resource
    ///
    /// This method is similar to `HttpRequest::url_for()` but it can be used
    /// for urls that do not contain variable parts.
    pub fn url_for_static(&self, name: &str) -> Result<Url, UrlGenerationError> {
        const NO_PARAMS: [&str; 0] = [];
        self.url_for(name, &NO_PARAMS)
    }

    /// This method returns reference to current `RouteInfo` object.
    #[inline]
    pub fn resource(&self) -> &ResourceInfo {
        &self.resource
    }

    /// Peer socket address
    ///
    /// Peer address is actual socket address, if proxy is used in front of
    /// actix http server, then peer address would be address of this proxy.
    ///
    /// To get client connection information `connection_info()` method should
    /// be used.
    #[inline]
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.request().inner.addr
    }

    /// url query parameters.
    pub fn query(&self) -> Ref<HashMap<String, String>> {
        if self.extensions().get::<Query>().is_none() {
            let mut query = HashMap::new();
            for (key, val) in form_urlencoded::parse(self.query_string().as_ref()) {
                query.insert(key.as_ref().to_string(), val.to_string());
            }
            self.extensions_mut().insert(Query(query));
        }
        Ref::map(self.extensions(), |ext| &ext.get::<Query>().unwrap().0)
    }

    /// The query string in the URL.
    ///
    /// E.g., id=10
    #[inline]
    pub fn query_string(&self) -> &str {
        if let Some(query) = self.uri().query().as_ref() {
            query
        } else {
            ""
        }
    }

    /// Load request cookies.
    #[inline]
    pub fn cookies(&self) -> Result<Ref<Vec<Cookie<'static>>>, CookieParseError> {
        if self.extensions().get::<Cookies>().is_none() {
            let mut cookies = Vec::new();
            for hdr in self.request().inner.headers.get_all(header::COOKIE) {
                let s = str::from_utf8(hdr.as_bytes()).map_err(CookieParseError::from)?;
                for cookie_str in s.split(';').map(|s| s.trim()) {
                    if !cookie_str.is_empty() {
                        cookies.push(Cookie::parse_encoded(cookie_str)?.into_owned());
                    }
                }
            }
            self.extensions_mut().insert(Cookies(cookies));
        }
        Ok(Ref::map(self.extensions(), |ext| {
            &ext.get::<Cookies>().unwrap().0
        }))
    }

    /// Return request cookie.
    #[inline]
    pub fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies.iter() {
                if cookie.name() == name {
                    return Some(cookie.to_owned());
                }
            }
        }
        None
    }

    pub(crate) fn set_cookies(&mut self, cookies: Option<Vec<Cookie<'static>>>) {
        if let Some(cookies) = cookies {
            self.extensions_mut().insert(Cookies(cookies));
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
        &self.resource.match_info()
    }

    /// Check if request requires connection upgrade
    pub(crate) fn upgrade(&self) -> bool {
        self.request().upgrade()
    }

    /// Set read buffer capacity
    ///
    /// Default buffer capacity is 32Kb.
    pub fn set_read_buffer_capacity(&mut self, cap: usize) {
        if let Some(payload) = self.request().inner.payload.borrow_mut().as_mut() {
            payload.set_read_buffer_capacity(cap)
        }
    }
}

impl<S> Drop for HttpRequest<S> {
    fn drop(&mut self) {
        if let Some(req) = self.req.take() {
            req.release();
        }
    }
}

impl<S> Clone for HttpRequest<S> {
    fn clone(&self) -> HttpRequest<S> {
        HttpRequest {
            req: self.req.as_ref().map(|r| r.clone()),
            state: self.state.clone(),
            resource: self.resource.clone(),
        }
    }
}

impl<S> FromRequest<S> for HttpRequest<S> {
    type Config = ();
    type Result = Self;

    #[inline]
    fn from_request(req: &HttpRequest<S>, _: &Self::Config) -> Self::Result {
        req.clone()
    }
}

impl<S> fmt::Debug for HttpRequest<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = writeln!(
            f,
            "\nHttpRequest {:?} {}:{}",
            self.version(),
            self.method(),
            self.path()
        );
        if !self.query_string().is_empty() {
            let _ = writeln!(f, "  query: ?{:?}", self.query_string());
        }
        if !self.match_info().is_empty() {
            let _ = writeln!(f, "  params: {:?}", self.match_info());
        }
        let _ = writeln!(f, "  headers:");
        for (key, val) in self.headers().iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resource::Resource;
    use router::{ResourceDef, Router};
    use test::TestRequest;

    #[test]
    fn test_debug() {
        let req = TestRequest::with_header("content-type", "text/plain").finish();
        let dbg = format!("{:?}", req);
        assert!(dbg.contains("HttpRequest"));
    }

    #[test]
    fn test_no_request_cookies() {
        let req = TestRequest::default().finish();
        assert!(req.cookies().unwrap().is_empty());
    }

    #[test]
    fn test_request_cookies() {
        let req = TestRequest::default()
            .header(header::COOKIE, "cookie1=value1")
            .header(header::COOKIE, "cookie2=value2")
            .finish();
        {
            let cookies = req.cookies().unwrap();
            assert_eq!(cookies.len(), 2);
            assert_eq!(cookies[0].name(), "cookie1");
            assert_eq!(cookies[0].value(), "value1");
            assert_eq!(cookies[1].name(), "cookie2");
            assert_eq!(cookies[1].value(), "value2");
        }

        let cookie = req.cookie("cookie1");
        assert!(cookie.is_some());
        let cookie = cookie.unwrap();
        assert_eq!(cookie.name(), "cookie1");
        assert_eq!(cookie.value(), "value1");

        let cookie = req.cookie("cookie-unknown");
        assert!(cookie.is_none());
    }

    #[test]
    fn test_request_query() {
        let req = TestRequest::with_uri("/?id=test").finish();
        assert_eq!(req.query_string(), "id=test");
        let query = req.query();
        assert_eq!(&query["id"], "test");
    }

    #[test]
    fn test_request_match_info() {
        let mut router = Router::<()>::default();
        router.register_resource(Resource::new(ResourceDef::new("/{key}/")));

        let req = TestRequest::with_uri("/value/?id=test").finish();
        let info = router.recognize(&req, &(), 0);
        assert_eq!(info.match_info().get("key"), Some("value"));
    }

    #[test]
    fn test_url_for() {
        let mut router = Router::<()>::default();
        let mut resource = Resource::new(ResourceDef::new("/user/{name}.{ext}"));
        resource.name("index");
        router.register_resource(resource);

        let info = router.default_route_info();
        assert!(!info.has_prefixed_resource("/use/"));
        assert!(info.has_resource("/user/test.html"));
        assert!(info.has_prefixed_resource("/user/test.html"));
        assert!(!info.has_resource("/test/unknown"));
        assert!(!info.has_prefixed_resource("/test/unknown"));

        let req = TestRequest::with_header(header::HOST, "www.rust-lang.org")
            .finish_with_router(router);

        assert_eq!(
            req.url_for("unknown", &["test"]),
            Err(UrlGenerationError::ResourceNotFound)
        );
        assert_eq!(
            req.url_for("index", &["test"]),
            Err(UrlGenerationError::NotEnoughElements)
        );
        let url = req.url_for("index", &["test", "html"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/user/test.html"
        );
    }

    #[test]
    fn test_url_for_with_prefix() {
        let mut resource = Resource::new(ResourceDef::new("/user/{name}.html"));
        resource.name("index");
        let mut router = Router::<()>::default();
        router.set_prefix("/prefix");
        router.register_resource(resource);

        let mut info = router.default_route_info();
        info.set_prefix(7);
        assert!(!info.has_prefixed_resource("/use/"));
        assert!(info.has_resource("/user/test.html"));
        assert!(!info.has_prefixed_resource("/user/test.html"));
        assert!(!info.has_resource("/prefix/user/test.html"));
        assert!(info.has_prefixed_resource("/prefix/user/test.html"));

        let req = TestRequest::with_uri("/prefix/test")
            .prefix(7)
            .header(header::HOST, "www.rust-lang.org")
            .finish_with_router(router);
        let url = req.url_for("index", &["test"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/prefix/user/test.html"
        );
    }

    #[test]
    fn test_url_for_static() {
        let mut resource = Resource::new(ResourceDef::new("/index.html"));
        resource.name("index");
        let mut router = Router::<()>::default();
        router.set_prefix("/prefix");
        router.register_resource(resource);

        let mut info = router.default_route_info();
        info.set_prefix(7);
        assert!(info.has_resource("/index.html"));
        assert!(!info.has_prefixed_resource("/index.html"));
        assert!(!info.has_resource("/prefix/index.html"));
        assert!(info.has_prefixed_resource("/prefix/index.html"));

        let req = TestRequest::with_uri("/prefix/test")
            .prefix(7)
            .header(header::HOST, "www.rust-lang.org")
            .finish_with_router(router);
        let url = req.url_for_static("index");
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/prefix/index.html"
        );
    }

    #[test]
    fn test_url_for_external() {
        let mut router = Router::<()>::default();
        router.register_external(
            "youtube",
            ResourceDef::external("https://youtube.com/watch/{video_id}"),
        );

        let info = router.default_route_info();
        assert!(!info.has_resource("https://youtube.com/watch/unknown"));
        assert!(!info.has_prefixed_resource("https://youtube.com/watch/unknown"));

        let req = TestRequest::default().finish_with_router(router);
        let url = req.url_for("youtube", &["oHg5SJYRHA0"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "https://youtube.com/watch/oHg5SJYRHA0"
        );
    }
}

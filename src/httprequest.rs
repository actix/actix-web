//! HTTP Request message related code.
#![cfg_attr(feature = "cargo-clippy", allow(transmute_ptr_to_ptr))]
use bytes::Bytes;
use cookie::Cookie;
use failure;
use futures::future::{result, FutureResult};
use futures::{Async, Poll, Stream};
use futures_cpupool::CpuPool;
use http::{header, Extensions, HeaderMap, Method, StatusCode, Uri, Version};
use std::net::SocketAddr;
use std::rc::Rc;
use std::{cmp, fmt, io, mem, str};
use tokio_io::AsyncRead;
use url::{form_urlencoded, Url};

use body::Body;
use error::{CookieParseError, Error, PayloadError, UrlGenerationError};
use handler::FromRequest;
use httpmessage::HttpMessage;
use httpresponse::{HttpResponse, HttpResponseBuilder};
use info::ConnectionInfo;
use param::Params;
use payload::Payload;
use router::{Resource, Router};
use server::helpers::SharedHttpInnerMessage;
use uri::Url as InnerUrl;

pub struct HttpInnerMessage {
    pub version: Version,
    pub method: Method,
    pub(crate) url: InnerUrl,
    pub headers: HeaderMap,
    pub extensions: Extensions,
    pub params: Params<'static>,
    pub cookies: Option<Vec<Cookie<'static>>>,
    pub query: Params<'static>,
    pub query_loaded: bool,
    pub addr: Option<SocketAddr>,
    pub payload: Option<Payload>,
    pub info: Option<ConnectionInfo<'static>>,
    pub keep_alive: bool,
    resource: RouterResource,
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum RouterResource {
    Notset,
    Normal(u16),
}

impl Default for HttpInnerMessage {
    fn default() -> HttpInnerMessage {
        HttpInnerMessage {
            method: Method::GET,
            url: InnerUrl::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            params: Params::new(),
            query: Params::new(),
            query_loaded: false,
            addr: None,
            cookies: None,
            payload: None,
            extensions: Extensions::new(),
            info: None,
            keep_alive: true,
            resource: RouterResource::Notset,
        }
    }
}

impl HttpInnerMessage {
    /// Checks if a connection should be kept alive.
    #[inline]
    pub fn keep_alive(&self) -> bool {
        self.keep_alive
    }

    #[inline]
    pub(crate) fn reset(&mut self) {
        self.headers.clear();
        self.extensions.clear();
        self.params.clear();
        self.addr = None;
        self.info = None;
        self.query_loaded = false;
        self.cookies = None;
        self.payload = None;
        self.keep_alive = true;
        self.resource = RouterResource::Notset;
    }
}

lazy_static! {
    static ref RESOURCE: Resource = Resource::unset();
}

/// An HTTP Request
pub struct HttpRequest<S = ()>(SharedHttpInnerMessage, Option<Rc<S>>, Option<Router>);

impl HttpRequest<()> {
    /// Construct a new Request.
    #[inline]
    pub fn new(
        method: Method, uri: Uri, version: Version, headers: HeaderMap,
        payload: Option<Payload>,
    ) -> HttpRequest {
        let url = InnerUrl::new(uri);
        HttpRequest(
            SharedHttpInnerMessage::from_message(HttpInnerMessage {
                method,
                url,
                version,
                headers,
                payload,
                params: Params::new(),
                query: Params::new(),
                query_loaded: false,
                extensions: Extensions::new(),
                cookies: None,
                addr: None,
                info: None,
                keep_alive: true,
                resource: RouterResource::Notset,
            }),
            None,
            None,
        )
    }

    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    pub(crate) fn from_message(msg: SharedHttpInnerMessage) -> HttpRequest {
        HttpRequest(msg, None, None)
    }

    #[inline]
    /// Construct new http request with state.
    pub fn with_state<S>(self, state: Rc<S>, router: Router) -> HttpRequest<S> {
        HttpRequest(self.0, Some(state), Some(router))
    }
}

impl<S> HttpMessage for HttpRequest<S> {
    #[inline]
    fn headers(&self) -> &HeaderMap {
        &self.as_ref().headers
    }
}

impl<S> HttpRequest<S> {
    #[inline]
    /// Construct new http request with state.
    pub fn change_state<NS>(&self, state: Rc<NS>) -> HttpRequest<NS> {
        HttpRequest(self.0.clone(), Some(state), self.2.clone())
    }

    #[inline]
    /// Construct new http request without state.
    pub fn drop_state(&self) -> HttpRequest {
        HttpRequest(self.0.clone(), None, self.2.clone())
    }

    /// get mutable reference for inner message
    /// mutable reference should not be returned as result for request's method
    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref, inline_always))]
    pub(crate) fn as_mut(&self) -> &mut HttpInnerMessage {
        self.0.get_mut()
    }

    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref, inline_always))]
    fn as_ref(&self) -> &HttpInnerMessage {
        self.0.get_ref()
    }

    #[inline]
    pub(crate) fn get_inner(&mut self) -> &mut HttpInnerMessage {
        self.as_mut()
    }

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        self.1.as_ref().unwrap()
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&mut self) -> &mut Extensions {
        &mut self.as_mut().extensions
    }

    /// Request extensions
    #[inline]
    #[doc(hidden)]
    pub fn extensions_ro(&self) -> &Extensions {
        &self.as_ref().extensions
    }

    /// Mutable refernece to a the request's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.as_mut().extensions
    }

    /// Default `CpuPool`
    #[inline]
    #[doc(hidden)]
    pub fn cpu_pool(&self) -> &CpuPool {
        self.router()
            .expect("HttpRequest has to have Router instance")
            .server_settings()
            .cpu_pool()
    }

    /// Create http response
    pub fn response(&self, status: StatusCode, body: Body) -> HttpResponse {
        if let Some(router) = self.router() {
            router.server_settings().get_response(status, body)
        } else {
            HttpResponse::with_body(status, body)
        }
    }

    /// Create http response builder
    pub fn build_response(&self, status: StatusCode) -> HttpResponseBuilder {
        if let Some(router) = self.router() {
            router.server_settings().get_response_builder(status)
        } else {
            HttpResponse::build(status)
        }
    }

    #[doc(hidden)]
    pub fn prefix_len(&self) -> usize {
        if let Some(router) = self.router() {
            router.prefix().len()
        } else {
            0
        }
    }

    /// Read the Request Uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        self.as_ref().url.uri()
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.as_ref().method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.as_ref().version
    }

    ///Returns mutable Request's headers.
    ///
    ///This is intended to be used by middleware.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.as_mut().headers
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.as_ref().url.path()
    }

    /// Get *ConnectionInfo* for correct request.
    pub fn connection_info(&self) -> &ConnectionInfo {
        if self.as_ref().info.is_none() {
            let info: ConnectionInfo<'static> =
                unsafe { mem::transmute(ConnectionInfo::new(self)) };
            self.as_mut().info = Some(info);
        }
        self.as_ref().info.as_ref().unwrap()
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
        if self.router().is_none() {
            Err(UrlGenerationError::RouterNotAvailable)
        } else {
            let path = self.router().unwrap().resource_path(name, elements)?;
            if path.starts_with('/') {
                let conn = self.connection_info();
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
    }

    /// This method returns reference to current `Router` object.
    #[inline]
    pub fn router(&self) -> Option<&Router> {
        self.2.as_ref()
    }

    /// This method returns reference to matched `Resource` object.
    #[inline]
    pub fn resource(&self) -> &Resource {
        if let Some(ref router) = self.2 {
            if let RouterResource::Normal(idx) = self.as_ref().resource {
                return router.get_resource(idx as usize);
            }
        }
        &*RESOURCE
    }

    pub(crate) fn set_resource(&mut self, res: usize) {
        self.as_mut().resource = RouterResource::Normal(res as u16);
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
        self.as_ref().addr
    }

    #[inline]
    pub(crate) fn set_peer_addr(&mut self, addr: Option<SocketAddr>) {
        self.as_mut().addr = addr;
    }

    /// Get a reference to the Params object.
    /// Params is a container for url query parameters.
    pub fn query(&self) -> &Params {
        if !self.as_ref().query_loaded {
            let params: &mut Params =
                unsafe { mem::transmute(&mut self.as_mut().query) };
            params.clear();
            self.as_mut().query_loaded = true;
            for (key, val) in form_urlencoded::parse(self.query_string().as_ref()) {
                params.add(key, val);
            }
        }
        unsafe { mem::transmute(&self.as_ref().query) }
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
    pub fn cookies(&self) -> Result<&Vec<Cookie<'static>>, CookieParseError> {
        if self.as_ref().cookies.is_none() {
            let msg = self.as_mut();
            let mut cookies = Vec::new();
            for hdr in msg.headers.get_all(header::COOKIE) {
                let s = str::from_utf8(hdr.as_bytes()).map_err(CookieParseError::from)?;
                for cookie_str in s.split(';').map(|s| s.trim()) {
                    if !cookie_str.is_empty() {
                        cookies.push(Cookie::parse_encoded(cookie_str)?.into_owned());
                    }
                }
            }
            msg.cookies = Some(cookies);
        }
        Ok(&self.as_ref().cookies.as_ref().unwrap())
    }

    /// Return request cookie.
    pub fn cookie(&self, name: &str) -> Option<&Cookie> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies {
                if cookie.name() == name {
                    return Some(cookie);
                }
            }
        }
        None
    }

    /// Get a reference to the Params object.
    ///
    /// Params is a container for url parameters.
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment.
    #[inline]
    pub fn match_info(&self) -> &Params {
        unsafe { mem::transmute(&self.as_ref().params) }
    }

    /// Get mutable reference to request's Params.
    #[inline]
    pub fn match_info_mut(&mut self) -> &mut Params {
        unsafe { mem::transmute(&mut self.as_mut().params) }
    }

    /// Checks if a connection should be kept alive.
    pub fn keep_alive(&self) -> bool {
        self.as_ref().keep_alive()
    }

    /// Check if request requires connection upgrade
    pub(crate) fn upgrade(&self) -> bool {
        if let Some(conn) = self.as_ref().headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade");
            }
        }
        self.as_ref().method == Method::CONNECT
    }

    /// Set read buffer capacity
    ///
    /// Default buffer capacity is 32Kb.
    pub fn set_read_buffer_capacity(&mut self, cap: usize) {
        if let Some(ref mut payload) = self.as_mut().payload {
            payload.set_read_buffer_capacity(cap)
        }
    }

    #[cfg(test)]
    pub(crate) fn payload(&self) -> &Payload {
        let msg = self.as_mut();
        if msg.payload.is_none() {
            msg.payload = Some(Payload::empty());
        }
        msg.payload.as_ref().unwrap()
    }

    #[cfg(test)]
    pub(crate) fn payload_mut(&mut self) -> &mut Payload {
        let msg = self.as_mut();
        if msg.payload.is_none() {
            msg.payload = Some(Payload::empty());
        }
        msg.payload.as_mut().unwrap()
    }
}

impl Default for HttpRequest<()> {
    /// Construct default request
    fn default() -> HttpRequest {
        HttpRequest(SharedHttpInnerMessage::default(), None, None)
    }
}

impl<S> Clone for HttpRequest<S> {
    fn clone(&self) -> HttpRequest<S> {
        HttpRequest(self.0.clone(), self.1.clone(), self.2.clone())
    }
}

impl<S: 'static> FromRequest<S> for HttpRequest<S> {
    type Config = ();
    type Result = FutureResult<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, _: &Self::Config) -> Self::Result {
        result(Ok(req.clone()))
    }
}

impl<S> Stream for HttpRequest<S> {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Bytes>, PayloadError> {
        let msg = self.as_mut();
        if msg.payload.is_none() {
            Ok(Async::Ready(None))
        } else {
            msg.payload.as_mut().unwrap().poll()
        }
    }
}

impl<S> io::Read for HttpRequest<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.as_mut().payload.is_some() {
            match self.as_mut().payload.as_mut().unwrap().poll() {
                Ok(Async::Ready(Some(mut b))) => {
                    let i = cmp::min(b.len(), buf.len());
                    buf.copy_from_slice(&b.split_to(i)[..i]);

                    if !b.is_empty() {
                        self.as_mut().payload.as_mut().unwrap().unread_data(b);
                    }

                    if i < buf.len() {
                        match self.read(&mut buf[i..]) {
                            Ok(n) => Ok(i + n),
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(i),
                            Err(e) => Err(e),
                        }
                    } else {
                        Ok(i)
                    }
                }
                Ok(Async::Ready(None)) => Ok(0),
                Ok(Async::NotReady) => {
                    Err(io::Error::new(io::ErrorKind::WouldBlock, "Not ready"))
                }
                Err(e) => Err(io::Error::new(
                    io::ErrorKind::Other,
                    failure::Error::from(e).compat(),
                )),
            }
        } else {
            Ok(0)
        }
    }
}

impl<S> AsyncRead for HttpRequest<S> {}

impl<S> fmt::Debug for HttpRequest<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = writeln!(
            f,
            "\nHttpRequest {:?} {}:{}",
            self.as_ref().version,
            self.as_ref().method,
            self.path()
        );
        if !self.query_string().is_empty() {
            let _ = writeln!(f, "  query: ?{:?}", self.query_string());
        }
        if !self.match_info().is_empty() {
            let _ = writeln!(f, "  params: {:?}", self.as_ref().params);
        }
        let _ = writeln!(f, "  headers:");
        for (key, val) in self.as_ref().headers.iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resource::ResourceHandler;
    use router::Resource;
    use server::ServerSettings;
    use test::TestRequest;

    #[test]
    fn test_debug() {
        let req = TestRequest::with_header("content-type", "text/plain").finish();
        let dbg = format!("{:?}", req);
        assert!(dbg.contains("HttpRequest"));
    }

    #[test]
    fn test_no_request_cookies() {
        let req = HttpRequest::default();
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
        let mut req = TestRequest::with_uri("/value/?id=test").finish();

        let mut resource = ResourceHandler::<()>::default();
        resource.name("index");
        let mut routes = Vec::new();
        routes.push((Resource::new("index", "/{key}/"), Some(resource)));
        let (router, _) = Router::new("", ServerSettings::default(), routes);
        assert!(router.recognize(&mut req).is_some());

        assert_eq!(req.match_info().get("key"), Some("value"));
    }

    #[test]
    fn test_url_for() {
        let req2 = HttpRequest::default();
        assert_eq!(
            req2.url_for("unknown", &["test"]),
            Err(UrlGenerationError::RouterNotAvailable)
        );

        let mut resource = ResourceHandler::<()>::default();
        resource.name("index");
        let routes = vec![(
            Resource::new("index", "/user/{name}.{ext}"),
            Some(resource),
        )];
        let (router, _) = Router::new("/", ServerSettings::default(), routes);
        assert!(router.has_route("/user/test.html"));
        assert!(!router.has_route("/test/unknown"));

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
        let req = TestRequest::with_header(header::HOST, "www.rust-lang.org").finish();

        let mut resource = ResourceHandler::<()>::default();
        resource.name("index");
        let routes = vec![(
            Resource::new("index", "/user/{name}.{ext}"),
            Some(resource),
        )];
        let (router, _) = Router::new("/prefix/", ServerSettings::default(), routes);
        assert!(router.has_route("/user/test.html"));
        assert!(!router.has_route("/prefix/user/test.html"));

        let req = req.with_state(Rc::new(()), router);
        let url = req.url_for("index", &["test", "html"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "http://www.rust-lang.org/prefix/user/test.html"
        );
    }

    #[test]
    fn test_url_for_external() {
        let req = HttpRequest::default();

        let mut resource = ResourceHandler::<()>::default();
        resource.name("index");
        let routes = vec![(
            Resource::external("youtube", "https://youtube.com/watch/{video_id}"),
            None,
        )];
        let (router, _) = Router::new::<()>("", ServerSettings::default(), routes);
        assert!(!router.has_route("https://youtube.com/watch/unknown"));

        let req = req.with_state(Rc::new(()), router);
        let url = req.url_for("youtube", &["oHg5SJYRHA0"]);
        assert_eq!(
            url.ok().unwrap().as_str(),
            "https://youtube.com/watch/oHg5SJYRHA0"
        );
    }
}

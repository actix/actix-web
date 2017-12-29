//! HTTP Request message related code.
use std::{str, fmt, mem};
use std::rc::Rc;
use std::net::SocketAddr;
use std::collections::HashMap;
use bytes::BytesMut;
use cookie::Cookie;
use futures::{Async, Future, Stream, Poll};
use http_range::HttpRange;
use serde::de::DeserializeOwned;
use url::{Url, form_urlencoded};
use http::{header, Uri, Method, Version, HeaderMap, Extensions};

use info::ConnectionInfo;
use param::Params;
use router::Router;
use payload::Payload;
use json::JsonBody;
use multipart::Multipart;
use helpers::SharedHttpMessage;
use error::{ParseError, UrlGenerationError, CookieParseError, HttpRangeError, UrlencodedError};


pub struct HttpMessage {
    pub version: Version,
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap,
    pub extensions: Extensions,
    pub params: Params<'static>,
    pub cookies: Option<Vec<Cookie<'static>>>,
    pub query: Params<'static>,
    pub query_loaded: bool,
    pub addr: Option<SocketAddr>,
    pub payload: Option<Payload>,
    pub info: Option<ConnectionInfo<'static>>,
}

impl Default for HttpMessage {

    fn default() -> HttpMessage {
        HttpMessage {
            method: Method::GET,
            uri: Uri::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            params: Params::new(),
            query: Params::new(),
            query_loaded: false,
            cookies: None,
            addr: None,
            payload: None,
            extensions: Extensions::new(),
            info: None,
        }
    }
}

impl HttpMessage {

    /// Checks if a connection should be kept alive.
    #[inline]
    pub fn keep_alive(&self) -> bool {
        if let Some(conn) = self.headers.get(header::CONNECTION) {
            if let Ok(conn) = conn.to_str() {
                if self.version == Version::HTTP_10 && conn.contains("keep-alive") {
                    true
                } else {
                    self.version == Version::HTTP_11 &&
                        !(conn.contains("close") || conn.contains("upgrade"))
                }
            } else {
                false
            }
        } else {
            self.version != Version::HTTP_10
        }
    }

    #[inline]
    pub(crate) fn reset(&mut self) {
        self.headers.clear();
        self.extensions.clear();
        self.params.clear();
        self.query.clear();
        self.query_loaded = false;
        self.cookies = None;
        self.addr = None;
        self.info = None;
        self.payload = None;
    }
}

/// An HTTP Request
pub struct HttpRequest<S=()>(SharedHttpMessage, Option<Rc<S>>, Option<Router>);

impl HttpRequest<()> {
    /// Construct a new Request.
    #[inline]
    pub fn new(method: Method, uri: Uri,
               version: Version, headers: HeaderMap, payload: Option<Payload>) -> HttpRequest
    {
        HttpRequest(
            SharedHttpMessage::from_message(HttpMessage {
                method: method,
                uri: uri,
                version: version,
                headers: headers,
                params: Params::new(),
                query: Params::new(),
                query_loaded: false,
                cookies: None,
                addr: None,
                payload: payload,
                extensions: Extensions::new(),
                info: None,
            }),
            None,
            None,
        )
    }

    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    pub(crate) fn from_message(msg: SharedHttpMessage) -> HttpRequest {
        HttpRequest(msg, None, None)
    }

    #[inline]
    /// Construct new http request with state.
    pub fn with_state<S>(self, state: Rc<S>, router: Router) -> HttpRequest<S> {
        HttpRequest(self.0, Some(state), Some(router))
    }
}

impl<S> HttpRequest<S> {

    #[inline]
    /// Construct new http request without state.
    pub(crate) fn clone_without_state(&self) -> HttpRequest {
        HttpRequest(self.0.clone(), None, None)
    }

    // get mutable reference for inner message
    // mutable reference should not be returned as result for request's method
    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref, inline_always))]
    pub(crate) fn as_mut(&self) -> &mut HttpMessage {
        self.0.get_mut()
    }

    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref, inline_always))]
    fn as_ref(&self) -> &HttpMessage {
        self.0.get_ref()
    }

    #[inline]
    pub(crate) fn get_inner(&mut self) -> &mut HttpMessage {
        self.as_mut()
    }

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        self.1.as_ref().unwrap()
    }

    /// Protocol extensions.
    #[inline]
    pub fn extensions(&mut self) -> &mut Extensions {
        &mut self.as_mut().extensions
    }

    #[doc(hidden)]
    pub fn prefix_len(&self) -> usize {
        if let Some(router) = self.router() { router.prefix().len() } else { 0 }
    }

    /// Read the Request Uri.
    #[inline]
    pub fn uri(&self) -> &Uri { &self.as_ref().uri }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method { &self.as_ref().method }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.as_ref().version
    }

    /// Read the Request Headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.as_ref().headers
    }

    #[doc(hidden)]
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.as_mut().headers
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.uri().path()
    }

    /// Get *ConnectionInfo* for currect request.
    pub fn connection_info(&self) -> &ConnectionInfo {
        if self.as_ref().info.is_none() {
            let info: ConnectionInfo<'static> = unsafe{
                mem::transmute(ConnectionInfo::new(self))};
            self.as_mut().info = Some(info);
        }
        self.as_ref().info.as_ref().unwrap()
    }

    /// Generate url for named resource
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # use actix_web::httpcodes::*;
    /// #
    /// fn index(req: HttpRequest) -> HttpResponse {
    ///     let url = req.url_for("foo", &["1", "2", "3"]); // <- generate url for "foo" resource
    ///     HTTPOk.into()
    /// }
    ///
    /// fn main() {
    ///     let app = Application::new()
    ///         .resource("/test/{one}/{two}/{three}", |r| {
    ///              r.name("foo");  // <- set resource name, then it could be used in `url_for`
    ///              r.method(Method::GET).f(|_| httpcodes::HTTPOk);
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn url_for<U, I>(&self, name: &str, elements: U) -> Result<Url, UrlGenerationError>
        where U: IntoIterator<Item=I>,
              I: AsRef<str>,
    {
        if self.router().is_none() {
            Err(UrlGenerationError::RouterNotAvailable)
        } else {
            let path = self.router().unwrap().resource_path(name, elements)?;
            if path.starts_with('/') {
                let conn = self.connection_info();
                Ok(Url::parse(&format!("{}://{}{}", conn.scheme(), conn.host(), path))?)
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

    /// Peer socket address
    ///
    /// Peer address is actuall socket address, if proxy is used in front of
    /// actix http server, then peer address would be address of this proxy.
    ///
    /// To get client connection information `connection_info()` method should be used.
    #[inline]
    pub fn peer_addr(&self) -> Option<&SocketAddr> {
        self.as_ref().addr.as_ref()
    }

    #[inline]
    pub(crate) fn set_peer_addr(&mut self, addr: Option<SocketAddr>) {
        self.as_mut().addr = addr
    }

    /// Get a reference to the Params object.
    /// Params is a container for url query parameters.
    pub fn query(&self) -> &Params {
        if !self.as_ref().query_loaded {
            let params: &mut Params = unsafe{ mem::transmute(&mut self.as_mut().query) };
            self.as_mut().query_loaded = true;
            for (key, val) in form_urlencoded::parse(self.query_string().as_ref()) {
                params.add(key, val);
            }
        }
        unsafe{ mem::transmute(&self.as_ref().query) }
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
            if let Some(val) = msg.headers.get(header::COOKIE) {
                let s = str::from_utf8(val.as_bytes())
                    .map_err(CookieParseError::from)?;
                for cookie in s.split("; ") {
                    cookies.push(Cookie::parse_encoded(cookie)?.into_owned());
                }
            }
            msg.cookies = Some(cookies)
        }
        Ok(self.as_ref().cookies.as_ref().unwrap())
    }

    /// Return request cookie.
    pub fn cookie(&self, name: &str) -> Option<&Cookie> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies {
                if cookie.name() == name {
                    return Some(cookie)
                }
            }
        }
        None
    }

    /// Get a reference to the Params object.
    /// Params is a container for url parameters.
    /// Route supports glob patterns: * for a single wildcard segment and :param
    /// for matching storing that segment of the request url in the Params object.
    #[inline]
    pub fn match_info(&self) -> &Params {
        unsafe{ mem::transmute(&self.as_ref().params) }
    }

    /// Get mutable reference to request's Params.
    #[inline]
    pub(crate) fn match_info_mut(&mut self) -> &mut Params {
        unsafe{ mem::transmute(&mut self.as_mut().params) }
    }

    /// Checks if a connection should be kept alive.
    pub fn keep_alive(&self) -> bool {
        self.as_ref().keep_alive()
    }

    /// Read the request content type. If request does not contain
    /// *Content-Type* header, empty str get returned.
    pub fn content_type(&self) -> &str {
        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return content_type
            }
        }
        ""
    }

    /// Check if request requires connection upgrade
    pub(crate) fn upgrade(&self) -> bool {
        if let Some(conn) = self.as_ref().headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade")
            }
        }
        self.as_ref().method == Method::CONNECT
    }

    /// Check if request has chunked transfer encoding
    pub fn chunked(&self) -> Result<bool, ParseError> {
        if let Some(encodings) = self.headers().get(header::TRANSFER_ENCODING) {
            if let Ok(s) = encodings.to_str() {
                Ok(s.to_lowercase().contains("chunked"))
            } else {
                Err(ParseError::Header)
            }
        } else {
            Ok(false)
        }
    }

    /// Parses Range HTTP header string as per RFC 2616.
    /// `size` is full size of response (file).
    pub fn range(&self, size: u64) -> Result<Vec<HttpRange>, HttpRangeError> {
        if let Some(range) = self.headers().get(header::RANGE) {
            HttpRange::parse(unsafe{str::from_utf8_unchecked(range.as_bytes())}, size)
                .map_err(|e| e.into())
        } else {
            Ok(Vec::new())
        }
    }

    /// Returns reference to the associated http payload.
    #[inline]
    pub fn payload(&self) -> &Payload {
        let msg = self.as_mut();
        if msg.payload.is_none() {
            msg.payload = Some(Payload::empty());
        }
        msg.payload.as_ref().unwrap()
    }

    /// Returns mutable reference to the associated http payload.
    #[inline]
    pub fn payload_mut(&mut self) -> &mut Payload {
        let msg = self.as_mut();
        if msg.payload.is_none() {
            msg.payload = Some(Payload::empty());
        }
        msg.payload.as_mut().unwrap()
    }

    /// Return stream to http payload processes as multipart.
    ///
    /// Content-type: multipart/form-data;
    ///
    /// ```rust
    /// # extern crate actix;
    /// # extern crate actix_web;
    /// # extern crate env_logger;
    /// # extern crate futures;
    /// # use std::str;
    /// # use actix::*;
    /// # use actix_web::*;
    /// # use futures::{Future, Stream};
    /// # use futures::future::{ok, result, Either};
    /// fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ///     req.multipart().from_err()       // <- get multipart stream for current request
    ///        .and_then(|item| match item { // <- iterate over multipart items
    ///            multipart::MultipartItem::Field(field) => {
    ///                // Field in turn is stream of *Bytes* object
    ///                Either::A(field.from_err()
    ///                          .map(|c| println!("-- CHUNK: \n{:?}", str::from_utf8(&c)))
    ///                          .finish())
    ///             },
    ///             multipart::MultipartItem::Nested(mp) => {
    ///                 // Or item could be nested Multipart stream
    ///                 Either::B(ok(()))
    ///             }
    ///         })
    ///         .finish()  // <- Stream::finish() combinator from actix
    ///         .map(|_| httpcodes::HTTPOk.response())
    ///         .responder()
    /// }
    /// # fn main() {}
    /// ```
    pub fn multipart(&mut self) -> Multipart {
        Multipart::from_request(self)
    }

    /// Parse `application/x-www-form-urlencoded` encoded body.
    /// Return `UrlEncoded` future. It resolves to a `HashMap<String, String>` which
    /// contains decoded parameters.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/x-www-form-urlencoded`
    /// * transfer encoding is `chunked`.
    /// * content-length is greater than 256k
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// use actix_web::*;
    /// use futures::future::{Future, ok};
    ///
    /// fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ///     req.urlencoded()         // <- get UrlEncoded future
    ///        .from_err()
    ///        .and_then(|params| {  // <- url encoded parameters
    ///             println!("==== BODY ==== {:?}", params);
    ///             ok(httpcodes::HTTPOk.response())
    ///        })
    ///        .responder()
    /// }
    /// # fn main() {}
    /// ```
    pub fn urlencoded(&mut self) -> UrlEncoded {
        UrlEncoded::from_request(self)
    }

    /// Parse `application/json` encoded body.
    /// Return `JsonBody<T>` future. It resolves to a `T` value.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/json`
    /// * content length is greater than 256k
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # #[macro_use] extern crate serde_derive;
    /// use actix_web::*;
    /// use futures::future::{Future, ok};
    ///
    /// #[derive(Deserialize, Debug)]
    /// struct MyObj {
    ///     name: String,
    /// }
    ///
    /// fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ///     req.json()                   // <- get JsonBody future
    ///        .from_err()
    ///        .and_then(|val: MyObj| {  // <- deserialized value
    ///            println!("==== BODY ==== {:?}", val);
    ///            Ok(httpcodes::HTTPOk.response())
    ///        }).responder()
    /// }
    /// # fn main() {}
    /// ```
    pub fn json<T: DeserializeOwned>(&mut self) -> JsonBody<S, T> {
        JsonBody::from_request(self)
    }
}

impl Default for HttpRequest<()> {

    /// Construct default request
    fn default() -> HttpRequest {
        HttpRequest(SharedHttpMessage::default(), None, None)
    }
}

impl<S> Clone for HttpRequest<S> {
    fn clone(&self) -> HttpRequest<S> {
        HttpRequest(self.0.clone(), self.1.clone(), None)
    }
}

impl<S> fmt::Debug for HttpRequest<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nHttpRequest {:?} {}:{}\n",
                         self.as_ref().version, self.as_ref().method, self.as_ref().uri);
        if !self.query_string().is_empty() {
            let _ = write!(f, "  query: ?{:?}\n", self.query_string());
        }
        if !self.match_info().is_empty() {
            let _ = write!(f, "  params: {:?}\n", self.as_ref().params);
        }
        let _ = write!(f, "  headers:\n");
        for key in self.as_ref().headers.keys() {
            let vals: Vec<_> = self.as_ref().headers.get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}

/// Future that resolves to a parsed urlencoded values.
pub struct UrlEncoded {
    pl: Payload,
    body: BytesMut,
    error: Option<UrlencodedError>,
}

impl UrlEncoded {
    pub fn from_request<S>(req: &mut HttpRequest<S>) -> UrlEncoded {
        let mut encoded = UrlEncoded {
            pl: req.payload_mut().clone(),
            body: BytesMut::new(),
            error: None
        };

        if let Ok(true) = req.chunked() {
            encoded.error = Some(UrlencodedError::Chunked);
        } else if let Some(len) = req.headers().get(header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    if len > 262_144 {
                        encoded.error = Some(UrlencodedError::Overflow);
                    }
                } else {
                    encoded.error = Some(UrlencodedError::UnknownLength);
                }
            } else {
                encoded.error = Some(UrlencodedError::UnknownLength);
            }
        }

        // check content type
        if encoded.error.is_none() {
            if let Some(content_type) = req.headers().get(header::CONTENT_TYPE) {
                if let Ok(content_type) = content_type.to_str() {
                    if content_type.to_lowercase() == "application/x-www-form-urlencoded" {
                        return encoded
                    }
                }
            }
            encoded.error = Some(UrlencodedError::ContentType);
            return encoded
        }

        encoded
    }
}

impl Future for UrlEncoded {
    type Item = HashMap<String, String>;
    type Error = UrlencodedError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(err) = self.error.take() {
            return Err(err)
        }

        loop {
            return match self.pl.poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(None)) => {
                    let mut m = HashMap::new();
                    for (k, v) in form_urlencoded::parse(&self.body) {
                        m.insert(k.into(), v.into());
                    }
                    Ok(Async::Ready(m))
                },
                Ok(Async::Ready(Some(item))) => {
                    self.body.extend_from_slice(&item);
                    continue
                },
                Err(err) => Err(err.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Uri;
    use std::str::FromStr;
    use router::Pattern;
    use resource::Resource;
    use test::TestRequest;
    use server::ServerSettings;

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
        let req = TestRequest::with_header(
            header::COOKIE, "cookie1=value1; cookie2=value2").finish();
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
    fn test_no_request_range_header() {
        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, HeaderMap::new(), None);
        let ranges = req.range(100).unwrap();
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_request_range_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE,
                       header::HeaderValue::from_static("bytes=0-4"));

        let req = HttpRequest::new(Method::GET, Uri::from_str("/").unwrap(),
                                   Version::HTTP_11, headers, None);
        let ranges = req.range(100).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].length, 5);
    }

    #[test]
    fn test_request_query() {
        let req = HttpRequest::new(Method::GET, Uri::from_str("/?id=test").unwrap(),
                                   Version::HTTP_11, HeaderMap::new(), None);
        assert_eq!(req.query_string(), "id=test");
        let query = req.query();
        assert_eq!(&query["id"], "test");
    }

    #[test]
    fn test_request_match_info() {
        let mut req = TestRequest::with_uri("/value/?id=test").finish();

        let mut resource = Resource::<()>::default();
        resource.name("index");
        let mut map = HashMap::new();
        map.insert(Pattern::new("index", "/{key}/"), Some(resource));
        let (router, _) = Router::new("", ServerSettings::default(), map);
        assert!(router.recognize(&mut req).is_some());

        assert_eq!(req.match_info().get("key"), Some("value"));
    }

    #[test]
    fn test_chunked() {
        let req = HttpRequest::default();
        assert!(!req.chunked().unwrap());

        let req = TestRequest::with_header(header::TRANSFER_ENCODING, "chunked").finish();
        assert!(req.chunked().unwrap());

        let mut headers = HeaderMap::new();
        let s = unsafe{str::from_utf8_unchecked(b"some va\xadscc\xacas0xsdasdlue".as_ref())};

        headers.insert(header::TRANSFER_ENCODING,
                       header::HeaderValue::from_str(s).unwrap());
        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, headers, None);
        assert!(req.chunked().is_err());
    }

    impl PartialEq for UrlencodedError {
        fn eq(&self, other: &UrlencodedError) -> bool {
            match *self {
                UrlencodedError::Chunked => match *other {
                    UrlencodedError::Chunked => true,
                    _ => false,
                },
                UrlencodedError::Overflow => match *other {
                    UrlencodedError::Overflow => true,
                    _ => false,
                },
                UrlencodedError::UnknownLength => match *other {
                    UrlencodedError::UnknownLength => true,
                    _ => false,
                },
                UrlencodedError::ContentType => match *other {
                    UrlencodedError::ContentType => true,
                    _ => false,
                },
                _ => false,
            }
        }
    }

    #[test]
    fn test_urlencoded_error() {
        let mut headers = HeaderMap::new();
        headers.insert(header::TRANSFER_ENCODING,
                       header::HeaderValue::from_static("chunked"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);

        assert_eq!(req.urlencoded().poll().err().unwrap(), UrlencodedError::Chunked);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("xxxx"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);

        assert_eq!(req.urlencoded().poll().err().unwrap(), UrlencodedError::UnknownLength);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("1000000"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);

        assert_eq!(req.urlencoded().poll().err().unwrap(), UrlencodedError::Overflow);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("text/plain"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("10"));
        let mut req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);

        assert_eq!(req.urlencoded().poll().err().unwrap(), UrlencodedError::ContentType);
    }

    #[test]
    fn test_url_for() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST,
                       header::HeaderValue::from_static("www.rust-lang.org"));
        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);

        let mut resource = Resource::<()>::default();
        resource.name("index");
        let mut map = HashMap::new();
        map.insert(Pattern::new("index", "/user/{name}.{ext}"), Some(resource));
        let (router, _) = Router::new("/", ServerSettings::default(), map);
        assert!(router.has_route("/user/test.html"));
        assert!(!router.has_route("/test/unknown"));

        assert_eq!(req.url_for("unknown", &["test"]),
                   Err(UrlGenerationError::RouterNotAvailable));

        let req = req.with_state(Rc::new(()), router);

        assert_eq!(req.url_for("unknown", &["test"]),
                   Err(UrlGenerationError::ResourceNotFound));
        assert_eq!(req.url_for("index", &["test"]),
                   Err(UrlGenerationError::NotEnoughElements));
        let url = req.url_for("index", &["test", "html"]);
        assert_eq!(url.ok().unwrap().as_str(), "http://www.rust-lang.org/user/test.html");
    }

    #[test]
    fn test_url_for_with_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST,
                       header::HeaderValue::from_static("www.rust-lang.org"));
        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);

        let mut resource = Resource::<()>::default();
        resource.name("index");
        let mut map = HashMap::new();
        map.insert(Pattern::new("index", "/user/{name}.{ext}"), Some(resource));
        let (router, _) = Router::new("/prefix/", ServerSettings::default(), map);
        assert!(router.has_route("/user/test.html"));
        assert!(!router.has_route("/prefix/user/test.html"));

        let req = req.with_state(Rc::new(()), router);
        let url = req.url_for("index", &["test", "html"]);
        assert_eq!(url.ok().unwrap().as_str(), "http://www.rust-lang.org/prefix/user/test.html");
    }

    #[test]
    fn test_url_for_external() {
        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(),
            Version::HTTP_11, HeaderMap::new(), None);

        let mut resource = Resource::<()>::default();
        resource.name("index");
        let mut map = HashMap::new();
        map.insert(Pattern::new("youtube", "https://youtube.com/watch/{video_id}"), None);
        let (router, _) = Router::new::<()>("", ServerSettings::default(), map);
        assert!(!router.has_route("https://youtube.com/watch/unknown"));

        let req = req.with_state(Rc::new(()), router);
        let url = req.url_for("youtube", &["oHg5SJYRHA0"]);
        assert_eq!(url.ok().unwrap().as_str(), "https://youtube.com/watch/oHg5SJYRHA0");
    }
}

//! HTTP Request message related code.
use std::{str, fmt, mem};
use std::rc::Rc;
use std::net::SocketAddr;
use std::collections::HashMap;
use bytes::BytesMut;
use futures::{Async, Future, Stream, Poll};
use url::form_urlencoded;
use http::{header, Method, Version, HeaderMap, Extensions};

use {Cookie, HttpRange};
use recognizer::Params;
use payload::Payload;
use multipart::Multipart;
use error::{ParseError, PayloadError,
            MultipartError, CookieParseError, HttpRangeError, UrlencodedError};

struct HttpMessage {
    version: Version,
    method: Method,
    path: String,
    query: String,
    headers: HeaderMap,
    extensions: Extensions,
    params: Params,
    cookies: Vec<Cookie<'static>>,
    cookies_loaded: bool,
    addr: Option<SocketAddr>,
    payload: Payload,
}

impl Default for HttpMessage {

    fn default() -> HttpMessage {
        HttpMessage {
            method: Method::GET,
            path: String::new(),
            query: String::new(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            params: Params::empty(),
            cookies: Vec::new(),
            cookies_loaded: false,
            addr: None,
            payload: Payload::empty(),
            extensions: Extensions::new(),
        }
    }
}

/// An HTTP Request
pub struct HttpRequest<S=()>(Rc<HttpMessage>, Rc<S>);

impl HttpRequest<()> {
    /// Construct a new Request.
    #[inline]
    pub fn new(method: Method, path: String, version: Version,
               headers: HeaderMap, query: String, payload: Payload) -> HttpRequest
    {
        HttpRequest(
            Rc::new(HttpMessage {
                method: method,
                path: path,
                query: query,
                version: version,
                headers: headers,
                params: Params::empty(),
                cookies: Vec::new(),
                cookies_loaded: false,
                addr: None,
                payload: payload,
                extensions: Extensions::new(),
            }),
            Rc::new(())
        )
    }

    /// Construct new http request with state.
    pub fn with_state<S>(self, state: Rc<S>) -> HttpRequest<S> {
        HttpRequest(self.0, state)
    }
}

impl<S> HttpRequest<S> {

    /// get mutable reference for inner message
    fn as_mut(&mut self) -> &mut HttpMessage {
        let r: &HttpMessage = self.0.as_ref();
        #[allow(mutable_transmutes)]
        unsafe{mem::transmute(r)}
    }

    /// Shared application state
    pub fn state(&self) -> &S {
        &self.1
    }

    /// Clone application state
    pub(crate) fn clone_state(&self) -> Rc<S> {
        Rc::clone(&self.1)
    }

    /// Protocol extensions.
    #[inline]
    pub fn extensions(&mut self) -> &mut Extensions {
        &mut self.as_mut().extensions
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method { &self.0.method }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.0.version
    }

    /// Read the Request Headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.0.headers
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        &self.0.path
    }

    /// Remote IP of client initiated HTTP request.
    ///
    /// The IP is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-For
    /// - peername of opened socket
    #[inline]
    pub fn remote(&self) -> Option<&SocketAddr> {
        self.0.addr.as_ref()
    }

    pub(crate) fn set_remove_addr(&mut self, addr: Option<SocketAddr>) {
        self.as_mut().addr = addr
    }

    /// Return a new iterator that yields pairs of `Cow<str>` for query parameters
    #[inline]
    pub fn query(&self) -> HashMap<String, String> {
        let mut q: HashMap<String, String> = HashMap::new();
        for (key, val) in form_urlencoded::parse(self.0.query.as_ref()) {
            q.insert(key.to_string(), val.to_string());
        }
        q
    }

    /// The query string in the URL.
    ///
    /// E.g., id=10
    #[inline]
    pub fn query_string(&self) -> &str {
        &self.0.query
    }

    /// Return request cookies.
    pub fn cookies(&self) -> &Vec<Cookie<'static>> {
        &self.0.cookies
    }

    /// Return request cookie.
    pub fn cookie(&self, name: &str) -> Option<&Cookie> {
        for cookie in &self.0.cookies {
            if cookie.name() == name {
                return Some(cookie)
            }
        }
        None
    }

    /// Load cookies
    pub fn load_cookies(&mut self) -> Result<&Vec<Cookie<'static>>, CookieParseError>
    {
        if !self.0.cookies_loaded {
            let msg = self.as_mut();
            msg.cookies_loaded = true;
            if let Some(val) = msg.headers.get(header::COOKIE) {
                let s = str::from_utf8(val.as_bytes())
                    .map_err(CookieParseError::from)?;
                for cookie in s.split("; ") {
                    msg.cookies.push(Cookie::parse_encoded(cookie)?.into_owned());
                }
            }
        }
        Ok(&self.0.cookies)
    }

    /// Get a reference to the Params object.
    /// Params is a container for url parameters.
    /// Route supports glob patterns: * for a single wildcard segment and :param
    /// for matching storing that segment of the request url in the Params object.
    #[inline]
    pub fn match_info(&self) -> &Params { &self.0.params }

    /// Set request Params.
    pub fn set_match_info(&mut self, params: Params) {
        self.as_mut().params = params;
    }

    /// Checks if a connection should be kept alive.
    pub fn keep_alive(&self) -> bool {
        if let Some(conn) = self.0.headers.get(header::CONNECTION) {
            if let Ok(conn) = conn.to_str() {
                if self.0.version == Version::HTTP_10 && conn.contains("keep-alive") {
                    true
                } else {
                    self.0.version == Version::HTTP_11 &&
                        !(conn.contains("close") || conn.contains("upgrade"))
                }
            } else {
                false
            }
        } else {
            self.0.version != Version::HTTP_10
        }
    }

    /// Read the request content type
    pub fn content_type(&self) -> &str {
        if let Some(content_type) = self.0.headers.get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return content_type
            }
        }
        ""
    }

    /// Check if request requires connection upgrade
    pub(crate) fn upgrade(&self) -> bool {
        if let Some(conn) = self.0.headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade")
            }
        }
        self.0.method == Method::CONNECT
    }

    /// Check if request has chunked transfer encoding
    pub fn chunked(&self) -> Result<bool, ParseError> {
        if let Some(encodings) = self.0.headers.get(header::TRANSFER_ENCODING) {
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
        if let Some(range) = self.0.headers.get(header::RANGE) {
            HttpRange::parse(unsafe{str::from_utf8_unchecked(range.as_bytes())}, size)
                .map_err(|e| e.into())
        } else {
            Ok(Vec::new())
        }
    }

    /// Returns reference to the associated http payload.
    #[inline]
    pub fn payload(&self) -> &Payload {
        &self.0.payload
    }

    /// Returns mutable reference to the associated http payload.
    #[inline]
    pub fn payload_mut(&mut self) -> &mut Payload {
        &mut self.as_mut().payload
    }

    /// Return payload
    pub fn take_payload(&mut self) -> Payload {
        mem::replace(&mut self.as_mut().payload, Payload::empty())
    }
    
    /// Return stream to process BODY as multipart.
    ///
    /// Content-type: multipart/form-data;
    pub fn multipart(&mut self) -> Result<Multipart, MultipartError> {
        let boundary = Multipart::boundary(&self.0.headers)?;
        Ok(Multipart::new(boundary, self.take_payload()))
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
    pub fn urlencoded(&mut self) -> Result<UrlEncoded, UrlencodedError> {
        if let Ok(true) = self.chunked() {
            return Err(UrlencodedError::Chunked)
        }

        if let Some(len) = self.headers().get(header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    if len > 262_144 {
                        return Err(UrlencodedError::Overflow)
                    }
                } else {
                    return Err(UrlencodedError::UnknownLength)
                }
            } else {
                return Err(UrlencodedError::UnknownLength)
            }
        }

        // check content type
        let t = if let Some(content_type) = self.0.headers.get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                content_type.to_lowercase() == "application/x-www-form-urlencoded"
            } else {
                false
            }
        } else {
            false
        };

        if t {
            Ok(UrlEncoded{pl: self.take_payload(), body: BytesMut::new()})
        } else {
            Err(UrlencodedError::ContentType)
        }
    }
}

impl Default for HttpRequest<()> {

    /// Construct default request
    fn default() -> HttpRequest {
        HttpRequest(Rc::new(HttpMessage::default()), Rc::new(()))
    }
}

impl<S> Clone for HttpRequest<S> {
    fn clone(&self) -> HttpRequest<S> {
        HttpRequest(Rc::clone(&self.0), Rc::clone(&self.1))
    }
}

impl<S> fmt::Debug for HttpRequest<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nHttpRequest {:?} {}:{}\n",
                         self.0.version, self.0.method, self.0.path);
        if !self.query_string().is_empty() {
            let _ = write!(f, "  query: ?{:?}\n", self.query_string());
        }
        if !self.0.params.is_empty() {
            let _ = write!(f, "  params: {:?}\n", self.0.params);
        }
        let _ = write!(f, "  headers:\n");
        for key in self.0.headers.keys() {
            let vals: Vec<_> = self.0.headers.get_all(key).iter().collect();
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
}

impl Future for UrlEncoded {
    type Item = HashMap<String, String>;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
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
                    self.body.extend(item.0);
                    continue
                },
                Err(err) => Err(err),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use payload::Payload;

    #[test]
    fn test_urlencoded_error() {
        let mut headers = HeaderMap::new();
        headers.insert(header::TRANSFER_ENCODING,
                       header::HeaderValue::from_static("chunked"));
        let mut req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new(), Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::Chunked);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("xxxx"));
        let mut req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new(), Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::UnknownLength);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("1000000"));
        let mut req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new(), Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::Overflow);

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("text/plain"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("10"));
        let mut req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new(), Payload::empty());

        assert_eq!(req.urlencoded().err().unwrap(), UrlencodedError::ContentType);
    }
}

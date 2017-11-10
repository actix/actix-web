//! HTTP Request message related code.
use std::{str, fmt};
use std::net::SocketAddr;
use std::collections::HashMap;
use bytes::BytesMut;
use futures::{Async, Future, Stream, Poll};
use url::form_urlencoded;
use http::{header, Method, Version, HeaderMap, Extensions};

use {Cookie, CookieParseError};
use {HttpRange, HttpRangeParseError};
use error::ParseError;
use recognizer::Params;
use payload::{Payload, PayloadError};
use multipart::{Multipart, MultipartError};


/// An HTTP Request
pub struct HttpRequest {
    version: Version,
    method: Method,
    path: String,
    query: String,
    headers: HeaderMap,
    params: Params,
    cookies: Vec<Cookie<'static>>,
    cookies_loaded: bool,
    extensions: Extensions,
    addr: Option<SocketAddr>,
}

impl HttpRequest {
    /// Construct a new Request.
    #[inline]
    pub fn new(method: Method, path: String,
               version: Version, headers: HeaderMap, query: String) -> Self
    {
        HttpRequest {
            method: method,
            path: path,
            query: query,
            version: version,
            headers: headers,
            params: Params::empty(),
            cookies: Vec::new(),
            cookies_loaded: false,
            extensions: Extensions::new(),
            addr: None,
        }
    }

    pub(crate) fn for_error() -> HttpRequest {
        HttpRequest {
            method: Method::GET,
            path: String::new(),
            query: String::new(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            params: Params::empty(),
            cookies: Vec::new(),
            cookies_loaded: false,
            extensions: Extensions::new(),
            addr: None,
        }
    }

    /// Protocol extensions.
    #[inline]
    pub fn extensions(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method { &self.method }

    /// Read the Request Version.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Read the Request Headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Remote IP of client initiated HTTP request.
    ///
    /// The IP is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-For
    /// - peername of opened socket
    pub fn remote(&self) -> Option<&SocketAddr> {
        self.addr.as_ref()
    }

    pub(crate) fn set_remove_addr(&mut self, addr: Option<SocketAddr>) {
        self.addr = addr
    }

    /// Return a new iterator that yields pairs of `Cow<str>` for query parameters
    #[inline]
    pub fn query(&self) -> HashMap<String, String> {
        let mut q: HashMap<String, String> = HashMap::new();
        for (key, val) in form_urlencoded::parse(self.query.as_ref()) {
            q.insert(key.to_string(), val.to_string());
        }
        q
    }

    /// The query string in the URL.
    ///
    /// E.g., id=10
    #[inline]
    pub fn query_string(&self) -> &str {
        &self.query
    }

    /// Return request cookies.
    pub fn cookies(&self) -> &Vec<Cookie<'static>> {
        &self.cookies
    }

    /// Return request cookie.
    pub fn cookie(&self, name: &str) -> Option<&Cookie> {
        for cookie in &self.cookies {
            if cookie.name() == name {
                return Some(cookie)
            }
        }
        None
    }

    /// Load cookies
    pub fn load_cookies(&mut self) -> Result<&Vec<Cookie>, CookieParseError>
    {
        if !self.cookies_loaded {
            self.cookies_loaded = true;
            if let Some(val) = self.headers.get(header::COOKIE) {
                let s = str::from_utf8(val.as_bytes())
                    .map_err(CookieParseError::from)?;
                for cookie in s.split("; ") {
                    self.cookies.push(Cookie::parse_encoded(cookie)?.into_owned());
                }
            }
        }
        Ok(&self.cookies)
    }

    /// Get a reference to the Params object.
    /// Params is a container for url parameters.
    /// Route supports glob patterns: * for a single wildcard segment and :param
    /// for matching storing that segment of the request url in the Params object.
    #[inline]
    pub fn match_info(&self) -> &Params { &self.params }

    /// Set request Params.
    pub fn set_match_info(&mut self, params: Params) {
        self.params = params;
    }

    /// Checks if a connection should be kept alive.
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

    /// Read the request content type
    pub fn content_type(&self) -> &str {
        if let Some(content_type) = self.headers.get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                return content_type
            }
        }
        ""
    }

    /// Check if request requires connection upgrade
    pub(crate) fn upgrade(&self) -> bool {
        if let Some(conn) = self.headers().get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade")
            }
        }
        self.method == Method::CONNECT
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
    pub fn range(&self, size: u64) -> Result<Vec<HttpRange>, HttpRangeParseError> {
        if let Some(range) = self.headers().get(header::RANGE) {
            HttpRange::parse(unsafe{str::from_utf8_unchecked(range.as_bytes())}, size)
        } else {
            Ok(Vec::new())
        }
    }

    /// Return stream to process BODY as multipart.
    ///
    /// Content-type: multipart/form-data;
    pub fn multipart(&self, payload: Payload) -> Result<Multipart, MultipartError> {
        Ok(Multipart::new(Multipart::boundary(&self.headers)?, payload))
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
    pub fn urlencoded(&self, payload: Payload) -> Result<UrlEncoded, Payload> {
        if let Ok(chunked) = self.chunked() {
            if chunked {
                return Err(payload)
            }
        }

        if let Some(len) = self.headers().get(header::CONTENT_LENGTH) {
            if let Ok(s) = len.to_str() {
                if let Ok(len) = s.parse::<u64>() {
                    if len > 262_144 {
                        return Err(payload)
                    }
                } else {
                    return Err(payload)
                }
            } else {
                return Err(payload)
            }
        }

        if let Some(content_type) = self.headers().get(header::CONTENT_TYPE) {
            if let Ok(content_type) = content_type.to_str() {
                if content_type.to_lowercase() == "application/x-www-form-urlencoded" {
                    return Ok(UrlEncoded{pl: payload, body: BytesMut::new()})
                }
            }
        }

        Err(payload)
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nHttpRequest {:?} {}:{}\n",
                         self.version, self.method, self.path);
        if !self.query_string().is_empty() {
            let _ = write!(f, "  query: ?{:?}\n", self.query_string());
        }
        if !self.params.is_empty() {
            let _ = write!(f, "  params: {:?}\n", self.params);
        }
        let _ = write!(f, "  headers:\n");
        for key in self.headers.keys() {
            let vals: Vec<_> = self.headers.get_all(key).iter().collect();
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
        let req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new());

        let (_, payload) = Payload::new(false);
        assert!(req.urlencoded(payload).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("xxxx"));
        let req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new());

        let (_, payload) = Payload::new(false);
        assert!(req.urlencoded(payload).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("application/x-www-form-urlencoded"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("1000000"));
        let req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new());

        let (_, payload) = Payload::new(false);
        assert!(req.urlencoded(payload).is_err());

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE,
                       header::HeaderValue::from_static("text/plain"));
        headers.insert(header::CONTENT_LENGTH,
                       header::HeaderValue::from_static("10"));
        let req = HttpRequest::new(
            Method::GET, "/".to_owned(), Version::HTTP_11, headers, String::new());

        let (_, payload) = Payload::new(false);
        assert!(req.urlencoded(payload).is_err());
    }

}

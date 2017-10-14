//! Pieces pertaining to the HTTP message protocol.
use std::{io, mem, str};
use std::convert::Into;

use cookie;
use bytes::Bytes;
use http::{Method, StatusCode, Version, Uri, HeaderMap, HttpTryFrom, Error};
use http::header::{self, HeaderName, HeaderValue};

use Params;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    Close,
    KeepAlive,
    Upgrade,
}

#[derive(Debug)]
/// An HTTP Request
pub struct HttpRequest {
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    params: Params,
    cookies: Vec<cookie::Cookie<'static>>,
}

impl HttpRequest {
    /// Construct a new Request.
    #[inline]
    pub fn new(method: Method, uri: Uri, version: Version, headers: HeaderMap) -> Self {
        HttpRequest {
            method: method,
            uri: uri,
            version: version,
            headers: headers,
            params: Params::new(),
            cookies: Vec::new(),
        }
    }

    /// Read the Request Uri.
    #[inline]
    pub fn uri(&self) -> &Uri { &self.uri }

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

    // /// The remote socket address of this request
    // ///
    // /// This is an `Option`, because some underlying transports may not have
    // /// a socket address, such as Unix Sockets.
    // ///
    // /// This field is not used for outgoing requests.
    // #[inline]
    // pub fn remote_addr(&self) -> Option<SocketAddr> { self.remote_addr }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.uri.path()
    }

    /// The query string of this Request.
    #[inline]
    pub fn query(&self) -> Option<&str> {
        self.uri.query()
    }

    /// Return request cookies.
    pub fn cookies(&mut self) -> &Vec<cookie::Cookie<'static>> {
        &self.cookies
    }

    /// Load cookies
    pub fn load_cookies(&mut self) -> Result<&Vec<cookie::Cookie>, cookie::ParseError>
    {
        if let Some(val) = self.headers.get(header::COOKIE) {
            let s = str::from_utf8(val.as_bytes())
                .map_err(cookie::ParseError::from)?;
            for cookie in s.split("; ") {
                self.cookies.push(cookie::Cookie::parse_encoded(cookie)?.into_owned());
            }
        }
        Ok(&self.cookies)
    }

    /// Get a mutable reference to the Request headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Get a reference to the Params object.
    /// Params is a container for url parameters.
    /// Route supports glob patterns: * for a single wildcard segment and :param
    /// for matching storing that segment of the request url in the Params object.
    #[inline]
    pub fn params(&self) -> &Params { &self.params }

    /// Create new request with Params object.
    pub fn with_params(self, params: Params) -> Self {
        HttpRequest {
            method: self.method,
            uri: self.uri,
            version: self.version,
            headers: self.headers,
            params: params,
            cookies: self.cookies,
        }
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

    pub(crate) fn upgrade(&self) -> bool {
        if let Some(conn) = self.headers().get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade")
            }
        }
        self.method == Method::CONNECT
    }

    pub fn chunked(&self) -> Result<bool, io::Error> {
        if let Some(encodings) = self.headers().get(header::TRANSFER_ENCODING) {
            if let Ok(s) = encodings.to_str() {
                Ok(s.to_lowercase().contains("chunked"))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other, "Can not read transfer-encoding header"))
            }
        } else {
            Ok(false)
        }
    }
}

/// Represents various types of http message body.
#[derive(Debug)]
pub enum Body {
    /// Empty response. `Content-Length` header is set to `0`
    Empty,
    /// Specific response body. `Content-Length` header is set to length of bytes.
    Binary(Bytes),
    /// Streaming response body with specified length.
    Length(u64),
    /// Unspecified streaming response. Developer is responsible for setting
    /// right `Content-Length` or `Transfer-Encoding` headers.
    Streaming,
    /// Upgrade connection.
    Upgrade,
}

impl Body {
    /// Does this body have payload.
    pub fn has_body(&self) -> bool {
        match *self {
            Body::Length(_) | Body::Streaming => true,
            _ => false
        }
    }
}

#[derive(Debug)]
/// An HTTP Response
pub struct HttpResponse {
    pub version: Option<Version>,
    pub headers: HeaderMap,
    pub status: StatusCode,
    reason: Option<&'static str>,
    body: Body,
    chunked: bool,
    connection_type: Option<ConnectionType>,
}

impl HttpResponse {

    #[inline]
    pub fn builder(status: StatusCode) -> Builder {
        Builder {
            parts: Some(Parts::new(status)),
            err: None,
        }
    }

    /// Constructs a response
    #[inline]
    pub fn new(status: StatusCode, body: Body) -> HttpResponse {
        HttpResponse {
            version: None,
            headers: Default::default(),
            status: status,
            reason: None,
            body: body,
            chunked: false,
            // compression: None,
            connection_type: None,
        }
    }

    /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&self) -> Option<Version> {
        self.version
    }

    /// Get the headers from the response.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a mutable reference to the headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Set the `StatusCode` for this response.
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.status
    }

    /// Get custom reason for the response.
    #[inline]
    pub fn reason(&self) -> &str {
        if let Some(ref reason) = self.reason {
            reason
        } else {
            ""
        }
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn set_reason(&mut self, reason: &'static str) -> &mut Self {
        self.reason = Some(reason);
        self
    }

    /// Set connection type
    pub fn set_connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        self.connection_type = Some(conn);
        self
    }

    /// Connection upgrade status
    pub fn upgrade(&self) -> bool {
        self.connection_type == Some(ConnectionType::Upgrade)
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> Option<bool> {
        if let Some(ConnectionType::KeepAlive) = self.connection_type {
            Some(true)
        } else {
            None
        }
    }

    /// is chunked encoding enabled
    pub fn chunked(&self) -> bool {
        self.chunked
    }

    /// Enables automatic chunked transfer encoding
    pub fn enable_chunked_encoding(&mut self) -> Result<(), io::Error> {
        if self.headers.contains_key(header::CONTENT_LENGTH) {
            Err(io::Error::new(io::ErrorKind::Other,
                "You can't enable chunked encoding when a content length is set"))
        } else {
            self.chunked = true;
            Ok(())
        }
    }

    /// Get body os this response
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// Set a body
    pub fn set_body<B: Into<Body>>(&mut self, body: B) {
        self.body = body.into();
    }

    /// Set a body and return previous body value
    pub fn replace_body<B: Into<Body>>(&mut self, body: B) -> Body {
        mem::replace(&mut self.body, body.into())
    }
}

/// Helper conversion implementation
impl<I: Into<HttpResponse>, E: Into<HttpResponse>> From<Result<I, E>> for HttpResponse {
    fn from(res: Result<I, E>) -> Self {
        match res {
            Ok(val) => val.into(),
            Err(err) => err.into(),
        }
    }
}

#[derive(Debug)]
struct Parts {
    version: Option<Version>,
    headers: HeaderMap,
    status: StatusCode,
    reason: Option<&'static str>,
    chunked: bool,
    connection_type: Option<ConnectionType>,
}

impl Parts {
    fn new(status: StatusCode) -> Self {
        Parts {
            version: None,
            headers: HeaderMap::new(),
            status: status,
            reason: None,
            chunked: false,
            connection_type: None,
        }
    }
}


/// An HTTP response builder
///
/// This type can be used to construct an instance of `HttpResponse` through a
/// builder-like pattern.
#[derive(Debug)]
pub struct Builder {
    parts: Option<Parts>,
    err: Option<Error>,
}

impl Builder {
    /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&mut self, version: Version) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            parts.version = Some(version);
        }
        self
    }

    /// Set the `StatusCode` for this response.
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            parts.status = status;
        }
        self
    }

    /// Set a header.
    #[inline]
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>,
              HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => {
                    match HeaderValue::try_from(value) {
                        Ok(value) => { parts.headers.append(key, value); }
                        Err(e) => self.err = Some(e.into()),
                    }
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn reason(&mut self, reason: &'static str) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            parts.reason = Some(reason);
        }
        self
    }

    /// Set connection type
    pub fn connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            parts.connection_type = Some(conn);
        }
        self
    }

    /// Set connection type to Upgrade
    pub fn upgrade(&mut self) -> &mut Self {
        self.connection_type(ConnectionType::Upgrade)
    }

    /// Force close connection, even if it is marked as keep-alive
    pub fn force_close(&mut self) -> &mut Self {
        self.connection_type(ConnectionType::Close)
    }

    /// Enables automatic chunked transfer encoding
    pub fn enable_chunked(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            parts.chunked = true;
        }
        self
    }

    /// Set response content type
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
        where HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => { parts.headers.insert(header::CONTENT_TYPE, value); },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /* /// Set response content charset
    pub fn charset<V>(&mut self, value: V) -> &mut Self
        where HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => { parts.headers.insert(header::CONTENT_TYPE, value); },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }*/

    /// Set a body
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Result<HttpResponse, Error> {
        let parts = self.parts.take().expect("cannot reuse response builder");
        if let Some(e) = self.err.take() {
            return Err(e)
        }
        Ok(HttpResponse {
            version: parts.version,
            headers: parts.headers,
            status: parts.status,
            reason: parts.reason,
            body: body.into(),
            chunked: parts.chunked,
            connection_type: parts.connection_type,
        })
    }
}

fn parts<'a>(parts: &'a mut Option<Parts>, err: &Option<Error>) -> Option<&'a mut Parts>
{
    if err.is_some() {
        return None
    }
    parts.as_mut()
}

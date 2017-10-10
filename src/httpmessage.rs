//! Pieces pertaining to the HTTP message protocol.
use std::{io, mem};
use std::convert::Into;

use bytes::Bytes;
use http::{Method, StatusCode, Version, Uri, HeaderMap};
use http::header::{self, HeaderName, HeaderValue};

use Params;
use error::Error;

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    Close,
    KeepAlive,
    Upgrade,
}

pub trait Message {

    fn version(&self) -> Version;

    fn headers(&self) -> &HeaderMap;

    /// Checks if a connection should be kept alive.
    fn keep_alive(&self) -> bool {
        if let Some(conn) = self.headers().get(header::CONNECTION) {
            if let Ok(conn) = conn.to_str() {
                if self.version() == Version::HTTP_10 && !conn.contains("keep-alive") {
                    false
                } else if self.version() == Version::HTTP_11 && conn.contains("close") {
                    false
                } else {
                    true
                }
            } else {
                false
            }
        } else {
            self.version() != Version::HTTP_10
        }
    }

    /// Checks if a connection is expecting a `100 Continue` before sending its body.
    #[inline]
    fn expecting_continue(&self) -> bool {
        if self.version() == Version::HTTP_11 {
            if let Some(hdr) = self.headers().get(header::EXPECT) {
                if let Ok(hdr) = hdr.to_str() {
                    return hdr.to_lowercase().contains("continue")
                }
            }
        }
        false
    }

    fn is_chunked(&self) -> Result<bool, Error> {
        if let Some(encodings) = self.headers().get(header::TRANSFER_ENCODING) {
            if let Ok(s) = encodings.to_str() {
                return Ok(s.to_lowercase().contains("chunked"))
            } else {
                debug!("request with transfer-encoding header, but not chunked, bad request");
                Err(Error::Header)
            }
        } else {
            Ok(false)
        }
    }
}


#[derive(Debug)]
/// An HTTP Request
pub struct HttpRequest {
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    params: Params,
}

impl Message for HttpRequest {
    fn version(&self) -> Version {
        self.version
    }
    fn headers(&self) -> &HeaderMap {
        &self.headers
    }
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
        }
    }

    /// Read the Request Uri.
    #[inline]
    pub fn uri(&self) -> &Uri { &self.uri }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version { self.version }

    /// Read the Request headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap { &self.headers }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method { &self.method }

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
            params: params
        }
    }

    pub(crate) fn is_upgrade(&self) -> bool {
        if let Some(ref conn) = self.headers().get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade")
            }
        }
        false
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

/// Implements by something that can be converted to `HttpResponse`
pub trait IntoHttpResponse {
    /// Convert into response.
    fn response(self, req: HttpRequest) -> HttpResponse;
}

#[derive(Debug)]
/// An HTTP Response
pub struct HttpResponse {
    request: HttpRequest,
    pub version: Version,
    pub headers: HeaderMap,
    pub status: StatusCode,
    reason: Option<&'static str>,
    body: Body,
    chunked: bool,
    // compression: Option<Encoding>,
    connection_type: Option<ConnectionType>,
}

impl Message for HttpResponse {
    fn version(&self) -> Version {
        self.version
    }
    fn headers(&self) -> &HeaderMap {
        &self.headers
    }
}

impl HttpResponse {
    /// Constructs a response
    #[inline]
    pub fn new(request: HttpRequest, status: StatusCode, body: Body) -> HttpResponse {
        let version = request.version;
        HttpResponse {
            request: request,
            version: version,
            headers: Default::default(),
            status: status,
            reason: None,
            body: body,
            chunked: false,
            // compression: None,
            connection_type: None,
        }
    }

    /// Original prequest
    #[inline]
    pub fn request(&self) -> &HttpRequest {
        &self.request
    }

    /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&self) -> Version {
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
    pub fn set_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    /// Set a header and move the Response.
    #[inline]
    pub fn set_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    /// Set the headers.
    #[inline]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn set_reason(mut self, reason: &'static str) -> Self {
        self.reason = Some(reason);
        self
    }

    /// Set connection type
    pub fn set_connection_type(mut self, conn: ConnectionType) -> Self {
        self.connection_type = Some(conn);
        self
    }

    /// Connection upgrade status
    pub fn upgrade(&self) -> bool {
        self.connection_type == Some(ConnectionType::Upgrade)
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> bool {
        if let Some(ConnectionType::KeepAlive) = self.connection_type {
            true
        } else {
            self.request.keep_alive()
        }
    }

    /// Force close connection, even if it is marked as keep-alive
    pub fn force_close(&mut self) {
        self.connection_type = Some(ConnectionType::Close);
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
    pub fn set_body<B: Into<Body>>(mut self, body: B) -> Self {
        self.body = body.into();
        self
    }

    /// Set a body and return previous body value
    pub fn replace_body<B: Into<Body>>(&mut self, body: B) -> Body {
        mem::replace(&mut self.body, body.into())
    }
}

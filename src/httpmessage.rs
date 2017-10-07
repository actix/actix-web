//! Pieces pertaining to the HTTP message protocol.
use std::{io, mem};
use std::str::FromStr;
use std::convert::Into;

use bytes::Bytes;
use http::{Method, StatusCode, Version, Uri};
use hyper::header::{Header, Headers};
use hyper::header::{Connection, ConnectionOption,
                    Expect, Encoding, ContentLength, TransferEncoding};

use Params;
use error::Error;

pub trait Message {

    fn version(&self) -> Version;

    fn headers(&self) -> &Headers;

    /// Checks if a connection should be kept alive.
    fn should_keep_alive(&self) -> bool {
        let ret = match (self.version(), self.headers().get::<Connection>()) {
            (Version::HTTP_10, None) => false,
            (Version::HTTP_10, Some(conn))
                if !conn.contains(&ConnectionOption::KeepAlive) => false,
            (Version::HTTP_11, Some(conn))
                if conn.contains(&ConnectionOption::Close)  => false,
            _ => true
        };
        trace!("should_keep_alive(version={:?}, header={:?}) = {:?}",
               self.version(), self.headers().get::<Connection>(), ret);
        ret
    }

    /// Checks if a connection is expecting a `100 Continue` before sending its body.
    #[inline]
    fn expecting_continue(&self) -> bool {
        let ret = match (self.version(), self.headers().get::<Expect>()) {
            (Version::HTTP_11, Some(&Expect::Continue)) => true,
            _ => false
        };
        trace!("expecting_continue(version={:?}, header={:?}) = {:?}",
               self.version(), self.headers().get::<Expect>(), ret);
        ret
    }

    fn is_chunked(&self) -> Result<bool, Error> {
        if let Some(&TransferEncoding(ref encodings)) = self.headers().get() {
            // https://tools.ietf.org/html/rfc7230#section-3.3.3
            // If Transfer-Encoding header is present, and 'chunked' is
            // not the final encoding, and this is a Request, then it is
            // mal-formed. A server should responsed with 400 Bad Request.
            if encodings.last() == Some(&Encoding::Chunked) {
                Ok(true)
            } else {
                debug!("request with transfer-encoding header, but not chunked, bad request");
                Err(Error::Header)
            }
        } else {
            Ok(false)
        }
    }

    fn is_upgrade(&self) -> bool {
        if let Some(&Connection(ref conn)) = self.headers().get() {
            conn.contains(&ConnectionOption::from_str("upgrade").unwrap())
        } else {
            false
        }
    }
}


#[derive(Debug)]
/// An HTTP Request
pub struct HttpRequest {
    version: Version,
    method: Method,
    uri: Uri,
    headers: Headers,
    params: Params,
}

impl Message for HttpRequest {
    fn version(&self) -> Version {
        self.version
    }
    fn headers(&self) -> &Headers {
        &self.headers
    }
}

impl HttpRequest {
    /// Construct a new Request.
    #[inline]
    pub fn new(method: Method, uri: Uri, version: Version, headers: Headers) -> Self {
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
    pub fn headers(&self) -> &Headers { &self.headers }

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
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    #[inline]
    pub fn params(&self) -> &Params { &self.params }

    pub fn with_params(self, params: Params) -> Self {
        HttpRequest {
            method: self.method,
            uri: self.uri,
            version: self.version,
            headers: self.headers,
            params: params
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

/// Implements by something that can be converted to `HttpMessage`
pub trait IntoHttpResponse {
    /// Convert into response.
    fn into_response(self, req: HttpRequest) -> HttpResponse;
}

#[derive(Debug)]
/// An HTTP Response
pub struct HttpResponse {
    request: HttpRequest,
    pub version: Version,
    pub headers: Headers,
    pub status: StatusCode,
    body: Body,
    chunked: bool,
    keep_alive: Option<bool>,
    compression: Option<Encoding>,
}

impl Message for HttpResponse {
    fn version(&self) -> Version {
        self.version
    }
    fn headers(&self) -> &Headers {
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
            body: body,
            chunked: false,
            keep_alive: None,
            compression: None,
        }
    }

    /// Get the HTTP version of this response.
    #[inline]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Get the headers from the response.
    #[inline]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Get a mutable reference to the headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Set the `StatusCode` for this response.
    #[inline]
    pub fn set_status(&mut self, status: StatusCode) -> &mut Self {
        self.status = status;
        self
    }

    /// Set a header and move the Response.
    #[inline]
    pub fn set_header<H: Header>(&mut self, header: H) -> &mut Self {
        self.headers.set(header);
        self
    }

    /// Set the headers and move the Response.
    #[inline]
    pub fn with_headers(&mut self, headers: Headers) -> &mut Self {
        self.headers = headers;
        self
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> bool {
        if let Some(ka) = self.keep_alive {
            ka
        } else {
            self.request.should_keep_alive()
        }
    }
    
    /// Force close connection, even if it is marked as keep-alive
    pub fn force_close(&mut self) {
        self.keep_alive = Some(false);
    }

    /// is chunked encoding enabled
    pub fn chunked(&self) -> bool {
        self.chunked
    }

    /// Enables automatic chunked transfer encoding
    pub fn enable_chunked_encoding(&mut self) -> Result<(), io::Error> {
        if self.headers.has::<ContentLength>() {
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

    /// Set a body and return previous body value
    pub fn set_body<B: Into<Body>>(&mut self, body: B) -> Body {
        mem::replace(&mut self.body, body.into())
    }
}

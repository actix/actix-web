//! Pieces pertaining to the HTTP response.
use std::{io, mem, str, fmt};
use std::convert::Into;

use cookie::CookieJar;
use http::{StatusCode, Version, HeaderMap, HttpTryFrom, Error as HttpError};
use http::header::{self, HeaderName, HeaderValue};

use Cookie;
use body::Body;
use route::Frame;
use encoding::ContentEncoding;

/// Represents various types of connection
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    /// Close connection after response
    Close,
    /// Keep connection alive after response
    KeepAlive,
    /// Connection is upgraded to different type
    Upgrade,
}

/// An HTTP Response
pub struct HttpResponse {
    pub version: Option<Version>,
    pub headers: HeaderMap,
    pub status: StatusCode,
    reason: Option<&'static str>,
    body: Body,
    chunked: bool,
    encoding: ContentEncoding,
    connection_type: Option<ConnectionType>,
    response_size: u64,
}

impl HttpResponse {

    #[inline]
    pub fn builder(status: StatusCode) -> HttpResponseBuilder {
        HttpResponseBuilder {
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
            encoding: ContentEncoding::Auto,
            connection_type: None,
            response_size: 0,
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
        if let Some(reason) = self.reason {
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
        if let Some(ct) = self.connection_type {
            match ct {
                ConnectionType::KeepAlive => Some(true),
                ConnectionType::Close | ConnectionType::Upgrade => Some(false),
            }
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

    /// Content encoding
    pub fn content_encoding(&self) -> &ContentEncoding {
        &self.encoding
    }

    /// Set content encoding
    pub fn set_content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        self.encoding = enc;
        self
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

    /// Size of response in bytes, excluding HTTP headers
    pub fn response_size(&self) -> u64 {
        self.response_size
    }

    /// Set content encoding
    pub(crate) fn set_response_size(&mut self, size: u64) {
        self.response_size = size;
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

impl From<HttpResponse> for Frame {
    fn from(resp: HttpResponse) -> Frame {
        Frame::Message(resp)
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nHttpResponse {:?} {}{}\n",
                         self.version, self.status, self.reason.unwrap_or(""));
        let _ = write!(f, "  encoding: {:?}\n", self.encoding);
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

#[derive(Debug)]
struct Parts {
    version: Option<Version>,
    headers: HeaderMap,
    status: StatusCode,
    reason: Option<&'static str>,
    chunked: bool,
    encoding: ContentEncoding,
    connection_type: Option<ConnectionType>,
    cookies: CookieJar,
}

impl Parts {
    fn new(status: StatusCode) -> Self {
        Parts {
            version: None,
            headers: HeaderMap::new(),
            status: status,
            reason: None,
            chunked: false,
            encoding: ContentEncoding::Auto,
            connection_type: None,
            cookies: CookieJar::new(),
        }
    }
}


/// An HTTP response builder
///
/// This type can be used to construct an instance of `HttpResponse` through a
/// builder-like pattern.
#[derive(Debug)]
pub struct HttpResponseBuilder {
    parts: Option<Parts>,
    err: Option<HttpError>,
}

impl HttpResponseBuilder {
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

    /// Set content encoding.
    ///
    /// By default `ContentEncoding::Auto` is used, which automatically
    /// negotiates content encoding based on request's `Accept-Encoding` headers.
    /// To enforce specific encodnign other `ContentEncoding` could be used.
    pub fn content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            parts.encoding = enc;
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

    /// Set a cookie
    pub fn cookie<'c>(&mut self, cookie: Cookie<'c>) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            parts.cookies.add(cookie.into_owned());
        }
        self
    }

    /// Remote cookie, cookie has to be cookie from `HttpRequest::cookies()` method.
    pub fn del_cookie<'a>(&mut self, cookie: &Cookie<'a>) -> &mut Self {
        if let Some(parts) = parts(&mut self.parts, &self.err) {
            let cookie = cookie.clone().into_owned();
            parts.cookies.add_original(cookie.clone());
            parts.cookies.remove(cookie);
        }
        self
    }

    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
        where F: Fn(&mut HttpResponseBuilder) + 'static
    {
        if value {
            f(self);
        }
        self
    }

    /// Set a body
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Result<HttpResponse, HttpError> {
        let mut parts = self.parts.take().expect("cannot reuse response builder");
        if let Some(e) = self.err.take() {
            return Err(e)
        }
        for cookie in parts.cookies.delta() {
            parts.headers.append(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie.to_string())?);
        }
        Ok(HttpResponse {
            version: parts.version,
            headers: parts.headers,
            status: parts.status,
            reason: parts.reason,
            body: body.into(),
            chunked: parts.chunked,
            encoding: parts.encoding,
            connection_type: parts.connection_type,
            response_size: 0,
        })
    }

    /// Set an empty body
    pub fn finish(&mut self) -> Result<HttpResponse, HttpError> {
        self.body(Body::Empty)
    }
}

fn parts<'a>(parts: &'a mut Option<Parts>, err: &Option<HttpError>) -> Option<&'a mut Parts>
{
    if err.is_some() {
        return None
    }
    parts.as_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_body() {
        assert!(Body::Length(10).has_body());
        assert!(Body::Streaming.has_body());
    }

    #[test]
    fn test_upgrade() {
        let resp = HttpResponse::builder(StatusCode::OK)
            .upgrade().body(Body::Empty).unwrap();
        assert!(resp.upgrade())
    }

    #[test]
    fn test_force_close() {
        let resp = HttpResponse::builder(StatusCode::OK)
            .force_close().body(Body::Empty).unwrap();
        assert!(!resp.keep_alive().unwrap())
    }

    #[test]
    fn test_content_type() {
        let resp = HttpResponse::builder(StatusCode::OK)
            .content_type("text/plain").body(Body::Empty).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "text/plain")
    }

    #[test]
    fn test_content_encoding() {
        let resp = HttpResponse::builder(StatusCode::OK).finish().unwrap();
        assert_eq!(*resp.content_encoding(), ContentEncoding::Auto);

        let resp = HttpResponse::builder(StatusCode::OK)
            .content_encoding(ContentEncoding::Br).finish().unwrap();
        assert_eq!(*resp.content_encoding(), ContentEncoding::Br);
    }
}

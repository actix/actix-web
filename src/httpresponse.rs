//! Http response
use std::{mem, str, fmt};
use std::io::Write;
use std::cell::RefCell;
use std::collections::VecDeque;

use cookie::{Cookie, CookieJar};
use bytes::{Bytes, BytesMut, BufMut};
use http::{StatusCode, Version, HeaderMap, HttpTryFrom, Error as HttpError};
use http::header::{self, HeaderName, HeaderValue};
use serde_json;
use serde::Serialize;

use body::Body;
use error::Error;
use handler::Responder;
use headers::ContentEncoding;
use httprequest::HttpRequest;

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
pub struct HttpResponse(Option<Box<InnerHttpResponse>>);

impl Drop for HttpResponse {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            Pool::release(inner)
        }
    }
}

impl HttpResponse {

    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    fn get_ref(&self) -> &InnerHttpResponse {
        self.0.as_ref().unwrap()
    }

    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    fn get_mut(&mut self) -> &mut InnerHttpResponse {
        self.0.as_mut().unwrap()
    }

    /// Create http response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> HttpResponseBuilder {
        HttpResponseBuilder {
            response: Some(Pool::get(status)),
            err: None,
            cookies: None,
        }
    }

    /// Constructs a response
    #[inline]
    pub fn new(status: StatusCode, body: Body) -> HttpResponse {
        HttpResponse(Some(Pool::with_body(status, body)))
    }

    /// Constructs a error response
    #[inline]
    pub fn from_error(error: Error) -> HttpResponse {
        let mut resp = error.cause().error_response();
        resp.get_mut().error = Some(error);
        resp
    }

    /// The source `error` for this response
    #[inline]
    pub fn error(&self) -> Option<&Error> {
        self.get_ref().error.as_ref()
    }

    /// Get the HTTP version of this response
    #[inline]
    pub fn version(&self) -> Option<Version> {
        self.get_ref().version
    }

    /// Get the headers from the response
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.get_ref().headers
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.get_mut().headers
    }

    /// Get the response status code
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.get_ref().status
    }

    /// Set the `StatusCode` for this response
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.get_mut().status
    }

    /// Get custom reason for the response
    #[inline]
    pub fn reason(&self) -> &str {
        if let Some(reason) = self.get_ref().reason {
            reason
        } else {
            self.get_ref().status.canonical_reason().unwrap_or("<unknown status code>")
        }
    }

    /// Set the custom reason for the response
    #[inline]
    pub fn set_reason(&mut self, reason: &'static str) -> &mut Self {
        self.get_mut().reason = Some(reason);
        self
    }

    /// Set connection type
    pub fn set_connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        self.get_mut().connection_type = Some(conn);
        self
    }

    /// Connection upgrade status
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.get_ref().connection_type == Some(ConnectionType::Upgrade)
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> Option<bool> {
        if let Some(ct) = self.get_ref().connection_type {
            match ct {
                ConnectionType::KeepAlive => Some(true),
                ConnectionType::Close | ConnectionType::Upgrade => Some(false),
            }
        } else {
            None
        }
    }

    /// is chunked encoding enabled
    #[inline]
    pub fn chunked(&self) -> Option<bool> {
        self.get_ref().chunked
    }

    /// Content encoding
    #[inline]
    pub fn content_encoding(&self) -> Option<ContentEncoding> {
        self.get_ref().encoding
    }

    /// Set content encoding
    pub fn set_content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        self.get_mut().encoding = Some(enc);
        self
    }

    /// Get body os this response
    #[inline]
    pub fn body(&self) -> &Body {
        &self.get_ref().body
    }

    /// Set a body
    pub fn set_body<B: Into<Body>>(&mut self, body: B) {
        self.get_mut().body = body.into();
    }

    /// Set a body and return previous body value
    pub fn replace_body<B: Into<Body>>(&mut self, body: B) -> Body {
        mem::replace(&mut self.get_mut().body, body.into())
    }

    /// Size of response in bytes, excluding HTTP headers
    pub fn response_size(&self) -> u64 {
        self.get_ref().response_size
    }

    /// Set content encoding
    pub(crate) fn set_response_size(&mut self, size: u64) {
        self.get_mut().response_size = size;
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = write!(f, "\nHttpResponse {:?} {}{}\n",
                         self.get_ref().version, self.get_ref().status,
                         self.get_ref().reason.unwrap_or(""));
        let _ = write!(f, "  encoding: {:?}\n", self.get_ref().encoding);
        let _ = write!(f, "  headers:\n");
        for key in self.get_ref().headers.keys() {
            let vals: Vec<_> = self.get_ref().headers.get_all(key).iter().collect();
            if vals.len() > 1 {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals);
            } else {
                let _ = write!(f, "    {:?}: {:?}\n", key, vals[0]);
            }
        }
        res
    }
}

/// An HTTP response builder
///
/// This type can be used to construct an instance of `HttpResponse` through a
/// builder-like pattern.
#[derive(Debug)]
pub struct HttpResponseBuilder {
    response: Option<Box<InnerHttpResponse>>,
    err: Option<HttpError>,
    cookies: Option<CookieJar>,
}

impl HttpResponseBuilder {
    /// Set HTTP version of this response.
    ///
    /// By default response's http version depends on request's version.
    #[inline]
    pub fn version(&mut self, version: Version) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.version = Some(version);
        }
        self
    }

    /// Set a header.
    ///
    /// ```rust
    /// # extern crate http;
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # use actix_web::httpcodes::*;
    /// #
    /// use http::header;
    ///
    /// fn index(req: HttpRequest) -> Result<HttpResponse> {
    ///     Ok(HttpOk.build()
    ///         .header("X-TEST", "value")
    ///         .header(header::CONTENT_TYPE, "application/json")
    ///         .finish()?)
    /// }
    /// fn main() {}
    /// ```
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
        where HeaderName: HttpTryFrom<K>,
              HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.response, &self.err) {
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
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.reason = Some(reason);
        }
        self
    }

    /// Set content encoding.
    ///
    /// By default `ContentEncoding::Auto` is used, which automatically
    /// negotiates content encoding based on request's `Accept-Encoding` headers.
    /// To enforce specific encoding, use specific ContentEncoding` value.
    #[inline]
    pub fn content_encoding(&mut self, enc: ContentEncoding) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.encoding = Some(enc);
        }
        self
    }

    /// Set connection type
    #[inline]
    #[doc(hidden)]
    pub fn connection_type(&mut self, conn: ConnectionType) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.connection_type = Some(conn);
        }
        self
    }

    /// Set connection type to Upgrade
    #[inline]
    #[doc(hidden)]
    pub fn upgrade(&mut self) -> &mut Self {
        self.connection_type(ConnectionType::Upgrade)
    }

    /// Force close connection, even if it is marked as keep-alive
    #[inline]
    pub fn force_close(&mut self) -> &mut Self {
        self.connection_type(ConnectionType::Close)
    }

    /// Enables automatic chunked transfer encoding
    #[inline]
    pub fn chunked(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.chunked = Some(true);
        }
        self
    }

    /// Force disable chunked encoding
    #[inline]
    pub fn no_chunking(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.chunked = Some(false);
        }
        self
    }

    /// Set response content type
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
        where HeaderValue: HttpTryFrom<V>
    {
        if let Some(parts) = parts(&mut self.response, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => { parts.headers.insert(header::CONTENT_TYPE, value); },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set content length
    #[inline]
    pub fn content_length(&mut self, len: u64) -> &mut Self {
        let mut wrt = BytesMut::new().writer();
        let _ = write!(wrt, "{}", len);
        self.header(header::CONTENT_LENGTH, wrt.get_mut().take().freeze())
    }

    /// Set a cookie
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # use actix_web::httpcodes::*;
    /// #
    /// use actix_web::headers::Cookie;
    ///
    /// fn index(req: HttpRequest) -> Result<HttpResponse> {
    ///     Ok(HttpOk.build()
    ///         .cookie(
    ///             Cookie::build("name", "value")
    ///                 .domain("www.rust-lang.org")
    ///                 .path("/")
    ///                 .secure(true)
    ///                 .http_only(true)
    ///                 .finish())
    ///         .finish()?)
    /// }
    /// fn main() {}
    /// ```
    pub fn cookie<'c>(&mut self, cookie: Cookie<'c>) -> &mut Self {
        if self.cookies.is_none() {
            let mut jar = CookieJar::new();
            jar.add(cookie.into_owned());
            self.cookies = Some(jar)
        } else {
            self.cookies.as_mut().unwrap().add(cookie.into_owned());
        }
        self
    }

    /// Remove cookie, cookie has to be cookie from `HttpRequest::cookies()` method.
    pub fn del_cookie<'a>(&mut self, cookie: &Cookie<'a>) -> &mut Self {
        {
            if self.cookies.is_none() {
                self.cookies = Some(CookieJar::new())
            }
            let jar = self.cookies.as_mut().unwrap();
            let cookie = cookie.clone().into_owned();
            jar.add_original(cookie.clone());
            jar.remove(cookie);
        }
        self
    }

    /// This method calls provided closure with builder reference if value is true.
    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
        where F: FnOnce(&mut HttpResponseBuilder)
    {
        if value {
            f(self);
        }
        self
    }

    /// This method calls provided closure with builder reference if value is Some.
    pub fn if_some<T, F>(&mut self, value: Option<T>, f: F) -> &mut Self
        where F: FnOnce(T, &mut HttpResponseBuilder)
    {
        if let Some(val) = value {
            f(val, self);
        }
        self
    }

    /// Set a body and generate `HttpResponse`.
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Result<HttpResponse, HttpError> {
        if let Some(e) = self.err.take() {
            return Err(e)
        }
        let mut response = self.response.take().expect("cannot reuse response builder");
        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                response.headers.append(
                    header::SET_COOKIE,
                    HeaderValue::from_str(&cookie.to_string())?);
            }
        }
        response.body = body.into();
        Ok(HttpResponse(Some(response)))
    }

    /// Set a json body and generate `HttpResponse`
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn json<T: Serialize>(&mut self, value: T) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&value)?;

        let contains = if let Some(parts) = parts(&mut self.response, &self.err) {
            parts.headers.contains_key(header::CONTENT_TYPE)
        } else {
            true
        };
        if !contains {
            self.header(header::CONTENT_TYPE, "application/json");
        }

        Ok(self.body(body)?)
    }

    /// Set an empty body and generate `HttpResponse`
    ///
    /// `HttpResponseBuilder` can not be used after this call.
    pub fn finish(&mut self) -> Result<HttpResponse, HttpError> {
        self.body(Body::Empty)
    }

    /// This method construct new `HttpResponseBuilder`
    pub fn take(&mut self) -> HttpResponseBuilder {
        HttpResponseBuilder {
            response: self.response.take(),
            err: self.err.take(),
            cookies: self.cookies.take(),
        }
    }
}

#[inline]
#[cfg_attr(feature = "cargo-clippy", allow(borrowed_box))]
fn parts<'a>(parts: &'a mut Option<Box<InnerHttpResponse>>, err: &Option<HttpError>)
             -> Option<&'a mut Box<InnerHttpResponse>>
{
    if err.is_some() {
        return None
    }
    parts.as_mut()
}

/// Helper converters
impl<I: Into<HttpResponse>, E: Into<Error>> From<Result<I, E>> for HttpResponse {
    fn from(res: Result<I, E>) -> Self {
        match res {
            Ok(val) => val.into(),
            Err(err) => err.into().into(),
        }
    }
}

impl From<HttpResponseBuilder> for HttpResponse {
    fn from(mut builder: HttpResponseBuilder) -> Self {
        builder.finish().into()
    }
}

impl Responder for HttpResponseBuilder {
    type Item = HttpResponse;
    type Error = HttpError;

    #[inline]
    fn respond_to(mut self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.finish()
    }
}

impl From<&'static str> for HttpResponse {
    fn from(val: &'static str) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(val)
            .into()
    }
}

impl Responder for &'static str {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self)
    }
}

impl From<&'static [u8]> for HttpResponse {
    fn from(val: &'static [u8]) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(val)
            .into()
    }
}

impl Responder for &'static [u8] {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self)
    }
}

impl From<String> for HttpResponse {
    fn from(val: String) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(val)
            .into()
    }
}

impl Responder for String {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self)
    }
}

impl<'a> From<&'a String> for HttpResponse {
    fn from(val: &'a String) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(val)
            .into()
    }
}

impl<'a> Responder for &'a String {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("text/plain; charset=utf-8")
            .body(self)
    }
}

impl From<Bytes> for HttpResponse {
    fn from(val: Bytes) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(val)
            .into()
    }
}

impl Responder for Bytes {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self)
    }
}

impl From<BytesMut> for HttpResponse {
    fn from(val: BytesMut) -> Self {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(val)
            .into()
    }
}

impl Responder for BytesMut {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        HttpResponse::build(StatusCode::OK)
            .content_type("application/octet-stream")
            .body(self)
    }
}

#[derive(Debug)]
struct InnerHttpResponse {
    version: Option<Version>,
    headers: HeaderMap,
    status: StatusCode,
    reason: Option<&'static str>,
    body: Body,
    chunked: Option<bool>,
    encoding: Option<ContentEncoding>,
    connection_type: Option<ConnectionType>,
    response_size: u64,
    error: Option<Error>,
}

impl InnerHttpResponse {

    #[inline]
    fn new(status: StatusCode, body: Body) -> InnerHttpResponse {
        InnerHttpResponse {
            status,
            body,
            version: None,
            headers: HeaderMap::with_capacity(16),
            reason: None,
            chunked: None,
            encoding: None,
            connection_type: None,
            response_size: 0,
            error: None,
        }
    }
}

/// Internal use only! unsafe
struct Pool(VecDeque<Box<InnerHttpResponse>>);

thread_local!(static POOL: RefCell<Pool> =
              RefCell::new(Pool(VecDeque::with_capacity(128))));

impl Pool {

    #[inline]
    fn get(status: StatusCode) -> Box<InnerHttpResponse> {
        POOL.with(|pool| {
            if let Some(mut resp) = pool.borrow_mut().0.pop_front() {
                resp.body = Body::Empty;
                resp.status = status;
                resp
            } else {
                Box::new(InnerHttpResponse::new(status, Body::Empty))
            }
        })
    }

    #[inline]
    fn with_body(status: StatusCode, body: Body) -> Box<InnerHttpResponse> {
        POOL.with(|pool| {
            if let Some(mut resp) = pool.borrow_mut().0.pop_front() {
                resp.status = status;
                resp.body = body;
                resp
            } else {
                Box::new(InnerHttpResponse::new(status, body))
            }
        })
    }

    #[inline(always)]
    #[cfg_attr(feature = "cargo-clippy", allow(boxed_local, inline_always))]
    fn release(mut inner: Box<InnerHttpResponse>) {
        POOL.with(|pool| {
            let v = &mut pool.borrow_mut().0;
            if v.len() < 128 {
                inner.headers.clear();
                inner.version = None;
                inner.chunked = None;
                inner.reason = None;
                inner.encoding = None;
                inner.connection_type = None;
                inner.response_size = 0;
                inner.error = None;
                v.push_front(inner);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use time::Duration;
    use http::{Method, Uri};
    use body::Binary;
    use {headers, httpcodes};

    #[test]
    fn test_debug() {
        let resp = HttpResponse::Ok().finish().unwrap();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("HttpResponse"));
    }

    #[test]
    fn test_response_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE,
                       header::HeaderValue::from_static("cookie1=value1; cookie2=value2"));

        let req = HttpRequest::new(
            Method::GET, Uri::from_str("/").unwrap(), Version::HTTP_11, headers, None);
        let cookies = req.cookies().unwrap();

        let resp = httpcodes::HttpOk
            .build()
            .cookie(headers::Cookie::build("name", "value")
                    .domain("www.rust-lang.org")
                    .path("/test")
                    .http_only(true)
                    .max_age(Duration::days(1))
                    .finish())
            .del_cookie(&cookies[0])
            .body(Body::Empty);

        assert!(resp.is_ok());
        let resp = resp.unwrap();

        let mut val: Vec<_> = resp.headers().get_all("Set-Cookie")
            .iter().map(|v| v.to_str().unwrap().to_owned()).collect();
        val.sort();
        assert!(val[0].starts_with("cookie1=; Max-Age=0;"));
        assert_eq!(
            val[1],"name=value; HttpOnly; Path=/test; Domain=www.rust-lang.org; Max-Age=86400");
    }

    #[test]
    fn test_basic_builder() {
        let resp = HttpResponse::Ok()
            .header("X-TEST", "value")
            .version(Version::HTTP_10)
            .finish().unwrap();
        assert_eq!(resp.version(), Some(Version::HTTP_10));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_upgrade() {
        let resp = HttpResponse::build(StatusCode::OK)
            .upgrade().body(Body::Empty).unwrap();
        assert!(resp.upgrade())
    }

    #[test]
    fn test_force_close() {
        let resp = HttpResponse::build(StatusCode::OK)
            .force_close().body(Body::Empty).unwrap();
        assert!(!resp.keep_alive().unwrap())
    }

    #[test]
    fn test_content_type() {
        let resp = HttpResponse::build(StatusCode::OK)
            .content_type("text/plain").body(Body::Empty).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "text/plain")
    }

    #[test]
    fn test_content_encoding() {
        let resp = HttpResponse::build(StatusCode::OK).finish().unwrap();
        assert_eq!(resp.content_encoding(), None);

        let resp = HttpResponse::build(StatusCode::OK)
            .content_encoding(ContentEncoding::Br).finish().unwrap();
        assert_eq!(resp.content_encoding(), Some(ContentEncoding::Br));
    }

    #[test]
    fn test_json() {
        let resp = HttpResponse::build(StatusCode::OK)
            .json(vec!["v1", "v2", "v3"]).unwrap();
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, header::HeaderValue::from_static("application/json"));
        assert_eq!(*resp.body(), Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]")));
    }

    #[test]
    fn test_json_ct() {
        let resp = HttpResponse::build(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/json")
            .json(vec!["v1", "v2", "v3"]).unwrap();
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
        assert_eq!(ct, header::HeaderValue::from_static("text/json"));
        assert_eq!(*resp.body(), Body::from(Bytes::from_static(b"[\"v1\",\"v2\",\"v3\"]")));
    }

    impl Body {
        pub(crate) fn binary(&self) -> Option<&Binary> {
            match *self {
                Body::Binary(ref bin) => Some(bin),
                _ => None,
            }
        }
    }

    #[test]
    fn test_into_response() {
        let req = HttpRequest::default();

        let resp: HttpResponse = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test"));

        let resp: HttpResponse = "test".respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test"));

        let resp: HttpResponse = b"test".as_ref().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(b"test".as_ref()));

        let resp: HttpResponse = b"test".as_ref().respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(b"test".as_ref()));

        let resp: HttpResponse = "test".to_owned().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test".to_owned()));

        let resp: HttpResponse = "test".to_owned().respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from("test".to_owned()));

        let resp: HttpResponse = (&"test".to_owned()).into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(&"test".to_owned()));

        let resp: HttpResponse = (&"test".to_owned()).respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("text/plain; charset=utf-8"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(&"test".to_owned()));

        let b = Bytes::from_static(b"test");
        let resp: HttpResponse = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(Bytes::from_static(b"test")));

        let b = Bytes::from_static(b"test");
        let resp: HttpResponse = b.respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(Bytes::from_static(b"test")));

        let b = BytesMut::from("test");
        let resp: HttpResponse = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(BytesMut::from("test")));

        let b = BytesMut::from("test");
        let resp: HttpResponse = b.respond_to(req.clone()).ok().unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(),
                   header::HeaderValue::from_static("application/octet-stream"));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().binary().unwrap(), &Binary::from(BytesMut::from("test")));
    }
}

//! Http response
use std::cell::{Ref, RefMut};
use std::io::Write;
use std::{fmt, str};

use bytes::{BufMut, Bytes, BytesMut};
use futures::future::{ok, FutureResult, IntoFuture};
use futures::Stream;
use serde::Serialize;
use serde_json;

use crate::body::{Body, BodyStream, MessageBody, ResponseBody};
use crate::cookie::{Cookie, CookieJar};
use crate::error::Error;
use crate::extensions::Extensions;
use crate::header::{Header, IntoHeaderValue};
use crate::http::header::{self, HeaderName, HeaderValue};
use crate::http::{Error as HttpError, HeaderMap, HttpTryFrom, StatusCode};
use crate::message::{BoxedResponseHead, ConnectionType, ResponseHead};

/// An HTTP Response
pub struct Response<B = Body> {
    head: BoxedResponseHead,
    body: ResponseBody<B>,
    error: Option<Error>,
}

impl Response<Body> {
    /// Create http response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> ResponseBuilder {
        ResponseBuilder::new(status)
    }

    /// Create http response builder
    #[inline]
    pub fn build_from<T: Into<ResponseBuilder>>(source: T) -> ResponseBuilder {
        source.into()
    }

    /// Constructs a response
    #[inline]
    pub fn new(status: StatusCode) -> Response {
        Response {
            head: BoxedResponseHead::new(status),
            body: ResponseBody::Body(Body::Empty),
            error: None,
        }
    }

    /// Constructs an error response
    #[inline]
    pub fn from_error(error: Error) -> Response {
        let mut resp = error.as_response_error().render_response();
        resp.error = Some(error);
        resp
    }

    /// Convert response to response with body
    pub fn into_body<B>(self) -> Response<B> {
        let b = match self.body {
            ResponseBody::Body(b) => b,
            ResponseBody::Other(b) => b,
        };
        Response {
            head: self.head,
            error: self.error,
            body: ResponseBody::Other(b),
        }
    }
}

impl<B> Response<B> {
    /// Constructs a response with body
    #[inline]
    pub fn with_body(status: StatusCode, body: B) -> Response<B> {
        Response {
            head: BoxedResponseHead::new(status),
            body: ResponseBody::Body(body),
            error: None,
        }
    }

    #[inline]
    /// Http message part of the response
    pub fn head(&self) -> &ResponseHead {
        &*self.head
    }

    #[inline]
    /// Mutable reference to a http message part of the response
    pub fn head_mut(&mut self) -> &mut ResponseHead {
        &mut *self.head
    }

    /// The source `error` for this response
    #[inline]
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    /// Get the response status code
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.head.status
    }

    /// Set the `StatusCode` for this response
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.head.status
    }

    /// Get the headers from the response
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head.headers
    }

    /// Get an iterator for the cookies set by this response
    #[inline]
    pub fn cookies(&self) -> CookieIter {
        CookieIter {
            iter: self.head.headers.get_all(header::SET_COOKIE),
        }
    }

    /// Add a cookie to this response
    #[inline]
    pub fn add_cookie(&mut self, cookie: &Cookie) -> Result<(), HttpError> {
        let h = &mut self.head.headers;
        HeaderValue::from_str(&cookie.to_string())
            .map(|c| {
                h.append(header::SET_COOKIE, c);
            })
            .map_err(|e| e.into())
    }

    /// Remove all cookies with the given name from this response. Returns
    /// the number of cookies removed.
    #[inline]
    pub fn del_cookie(&mut self, name: &str) -> usize {
        let h = &mut self.head.headers;
        let vals: Vec<HeaderValue> = h
            .get_all(header::SET_COOKIE)
            .map(|v| v.to_owned())
            .collect();
        h.remove(header::SET_COOKIE);

        let mut count: usize = 0;
        for v in vals {
            if let Ok(s) = v.to_str() {
                if let Ok(c) = Cookie::parse_encoded(s) {
                    if c.name() == name {
                        count += 1;
                        continue;
                    }
                }
            }
            h.append(header::SET_COOKIE, v);
        }
        count
    }

    /// Connection upgrade status
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.head.upgrade()
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> bool {
        self.head.keep_alive()
    }

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.head.extensions.borrow()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<Extensions> {
        self.head.extensions.borrow_mut()
    }

    /// Get body os this response
    #[inline]
    pub fn body(&self) -> &ResponseBody<B> {
        &self.body
    }

    /// Set a body
    pub fn set_body<B2>(self, body: B2) -> Response<B2> {
        Response {
            head: self.head,
            body: ResponseBody::Body(body),
            error: None,
        }
    }

    /// Split response and body
    pub fn into_parts(self) -> (Response<()>, ResponseBody<B>) {
        (
            Response {
                head: self.head,
                body: ResponseBody::Body(()),
                error: self.error,
            },
            self.body,
        )
    }

    /// Drop request's body
    pub fn drop_body(self) -> Response<()> {
        Response {
            head: self.head,
            body: ResponseBody::Body(()),
            error: None,
        }
    }

    /// Set a body and return previous body value
    pub(crate) fn replace_body<B2>(self, body: B2) -> (Response<B2>, ResponseBody<B>) {
        (
            Response {
                head: self.head,
                body: ResponseBody::Body(body),
                error: self.error,
            },
            self.body,
        )
    }

    /// Set a body and return previous body value
    pub fn map_body<F, B2>(mut self, f: F) -> Response<B2>
    where
        F: FnOnce(&mut ResponseHead, ResponseBody<B>) -> ResponseBody<B2>,
    {
        let body = f(&mut self.head, self.body);

        Response {
            body,
            head: self.head,
            error: self.error,
        }
    }

    /// Extract response body
    pub fn take_body(&mut self) -> ResponseBody<B> {
        self.body.take_body()
    }
}

impl<B: MessageBody> fmt::Debug for Response<B> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = writeln!(
            f,
            "\nResponse {:?} {}{}",
            self.head.version,
            self.head.status,
            self.head.reason.unwrap_or(""),
        );
        let _ = writeln!(f, "  headers:");
        for (key, val) in self.head.headers.iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        let _ = writeln!(f, "  body: {:?}", self.body.size());
        res
    }
}

impl IntoFuture for Response {
    type Item = Response;
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn into_future(self) -> Self::Future {
        ok(self)
    }
}

pub struct CookieIter<'a> {
    iter: header::GetAll<'a>,
}

impl<'a> Iterator for CookieIter<'a> {
    type Item = Cookie<'a>;

    #[inline]
    fn next(&mut self) -> Option<Cookie<'a>> {
        for v in self.iter.by_ref() {
            if let Ok(c) = Cookie::parse_encoded(v.to_str().ok()?) {
                return Some(c);
            }
        }
        None
    }
}

/// An HTTP response builder
///
/// This type can be used to construct an instance of `Response` through a
/// builder-like pattern.
pub struct ResponseBuilder {
    head: Option<BoxedResponseHead>,
    err: Option<HttpError>,
    cookies: Option<CookieJar>,
}

impl ResponseBuilder {
    #[inline]
    /// Create response builder
    pub fn new(status: StatusCode) -> Self {
        ResponseBuilder {
            head: Some(BoxedResponseHead::new(status)),
            err: None,
            cookies: None,
        }
    }

    /// Set HTTP status code of this response.
    #[inline]
    pub fn status(&mut self, status: StatusCode) -> &mut Self {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            parts.status = status;
        }
        self
    }

    /// Set a header.
    ///
    /// ```rust
    /// use actix_http::{http, Request, Response, Result};
    ///
    /// fn index(req: Request) -> Result<Response> {
    ///     Ok(Response::Ok()
    ///         .set(http::header::IfModifiedSince(
    ///             "Sun, 07 Nov 1994 08:48:37 GMT".parse()?,
    ///         ))
    ///         .finish())
    /// }
    /// fn main() {}
    /// ```
    #[doc(hidden)]
    pub fn set<H: Header>(&mut self, hdr: H) -> &mut Self {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            match hdr.try_into() {
                Ok(value) => {
                    parts.headers.append(H::name(), value);
                }
                Err(e) => self.err = Some(e.into()),
            }
        }
        self
    }

    /// Append a header to existing headers.
    ///
    /// ```rust
    /// use actix_http::{http, Request, Response};
    ///
    /// fn index(req: Request) -> Response {
    ///     Response::Ok()
    ///         .header("X-TEST", "value")
    ///         .header(http::header::CONTENT_TYPE, "application/json")
    ///         .finish()
    /// }
    /// fn main() {}
    /// ```
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => match value.try_into() {
                    Ok(value) => {
                        parts.headers.append(key, value);
                    }
                    Err(e) => self.err = Some(e.into()),
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set a header.
    ///
    /// ```rust
    /// use actix_http::{http, Request, Response};
    ///
    /// fn index(req: Request) -> Response {
    ///     Response::Ok()
    ///         .set_header("X-TEST", "value")
    ///         .set_header(http::header::CONTENT_TYPE, "application/json")
    ///         .finish()
    /// }
    /// fn main() {}
    /// ```
    pub fn set_header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            match HeaderName::try_from(key) {
                Ok(key) => match value.try_into() {
                    Ok(value) => {
                        parts.headers.insert(key, value);
                    }
                    Err(e) => self.err = Some(e.into()),
                },
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Set the custom reason for the response.
    #[inline]
    pub fn reason(&mut self, reason: &'static str) -> &mut Self {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            parts.reason = Some(reason);
        }
        self
    }

    /// Set connection type to KeepAlive
    #[inline]
    pub fn keep_alive(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            parts.set_connection_type(ConnectionType::KeepAlive);
        }
        self
    }

    /// Set connection type to Upgrade
    #[inline]
    pub fn upgrade<V>(&mut self, value: V) -> &mut Self
    where
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            parts.set_connection_type(ConnectionType::Upgrade);
        }
        self.set_header(header::UPGRADE, value)
    }

    /// Force close connection, even if it is marked as keep-alive
    #[inline]
    pub fn force_close(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            parts.set_connection_type(ConnectionType::Close);
        }
        self
    }

    /// Disable chunked transfer encoding for HTTP/1.1 streaming responses.
    #[inline]
    pub fn no_chunking(&mut self) -> &mut Self {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            parts.no_chunking(true);
        }
        self
    }

    /// Set response content type
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
    where
        HeaderValue: HttpTryFrom<V>,
    {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            match HeaderValue::try_from(value) {
                Ok(value) => {
                    parts.headers.insert(header::CONTENT_TYPE, value);
                }
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
    /// use actix_http::{http, Request, Response};
    ///
    /// fn index(req: Request) -> Response {
    ///     Response::Ok()
    ///         .cookie(
    ///             http::Cookie::build("name", "value")
    ///                 .domain("www.rust-lang.org")
    ///                 .path("/")
    ///                 .secure(true)
    ///                 .http_only(true)
    ///                 .finish(),
    ///         )
    ///         .finish()
    /// }
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

    /// Remove cookie
    ///
    /// ```rust
    /// use actix_http::{http, Request, Response, HttpMessage};
    ///
    /// fn index(req: Request) -> Response {
    ///     let mut builder = Response::Ok();
    ///
    ///     if let Some(ref cookie) = req.cookie("name") {
    ///         builder.del_cookie(cookie);
    ///     }
    ///
    ///     builder.finish()
    /// }
    /// ```
    pub fn del_cookie<'a>(&mut self, cookie: &Cookie<'a>) -> &mut Self {
        if self.cookies.is_none() {
            self.cookies = Some(CookieJar::new())
        }
        let jar = self.cookies.as_mut().unwrap();
        let cookie = cookie.clone().into_owned();
        jar.add_original(cookie.clone());
        jar.remove(cookie);
        self
    }

    /// This method calls provided closure with builder reference if value is
    /// true.
    pub fn if_true<F>(&mut self, value: bool, f: F) -> &mut Self
    where
        F: FnOnce(&mut ResponseBuilder),
    {
        if value {
            f(self);
        }
        self
    }

    /// This method calls provided closure with builder reference if value is
    /// Some.
    pub fn if_some<T, F>(&mut self, value: Option<T>, f: F) -> &mut Self
    where
        F: FnOnce(T, &mut ResponseBuilder),
    {
        if let Some(val) = value {
            f(val, self);
        }
        self
    }

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions.borrow()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions.borrow_mut()
    }

    #[inline]
    /// Set a body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Response {
        self.message_body(body.into())
    }

    /// Set a body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn message_body<B>(&mut self, body: B) -> Response<B> {
        if let Some(e) = self.err.take() {
            return Response::from(Error::from(e)).into_body();
        }

        let mut response = self.head.take().expect("cannot reuse response builder");

        if let Some(ref jar) = self.cookies {
            for cookie in jar.delta() {
                match HeaderValue::from_str(&cookie.to_string()) {
                    Ok(val) => response.headers.append(header::SET_COOKIE, val),
                    Err(e) => return Response::from(Error::from(e)).into_body(),
                };
            }
        }

        Response {
            head: response,
            body: ResponseBody::Body(body),
            error: None,
        }
    }

    #[inline]
    /// Set a streaming body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn streaming<S, E>(&mut self, stream: S) -> Response
    where
        S: Stream<Item = Bytes, Error = E> + 'static,
        E: Into<Error> + 'static,
    {
        self.body(Body::from_message(BodyStream::new(stream)))
    }

    #[inline]
    /// Set a json body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn json<T: Serialize>(&mut self, value: T) -> Response {
        self.json2(&value)
    }

    /// Set a json body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn json2<T: Serialize>(&mut self, value: &T) -> Response {
        match serde_json::to_string(value) {
            Ok(body) => {
                let contains = if let Some(parts) = parts(&mut self.head, &self.err) {
                    parts.headers.contains_key(header::CONTENT_TYPE)
                } else {
                    true
                };
                if !contains {
                    self.header(header::CONTENT_TYPE, "application/json");
                }

                self.body(Body::from(body))
            }
            Err(e) => Error::from(e).into(),
        }
    }

    #[inline]
    /// Set an empty body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn finish(&mut self) -> Response {
        self.body(Body::Empty)
    }

    /// This method construct new `ResponseBuilder`
    pub fn take(&mut self) -> ResponseBuilder {
        ResponseBuilder {
            head: self.head.take(),
            err: self.err.take(),
            cookies: self.cookies.take(),
        }
    }
}

#[inline]
fn parts<'a>(
    parts: &'a mut Option<BoxedResponseHead>,
    err: &Option<HttpError>,
) -> Option<&'a mut ResponseHead> {
    if err.is_some() {
        return None;
    }
    parts.as_mut().map(|r| &mut **r)
}

/// Convert `Response` to a `ResponseBuilder`. Body get dropped.
impl<B> From<Response<B>> for ResponseBuilder {
    fn from(res: Response<B>) -> ResponseBuilder {
        // If this response has cookies, load them into a jar
        let mut jar: Option<CookieJar> = None;
        for c in res.cookies() {
            if let Some(ref mut j) = jar {
                j.add_original(c.into_owned());
            } else {
                let mut j = CookieJar::new();
                j.add_original(c.into_owned());
                jar = Some(j);
            }
        }

        ResponseBuilder {
            head: Some(res.head),
            err: None,
            cookies: jar,
        }
    }
}

/// Convert `ResponseHead` to a `ResponseBuilder`
impl<'a> From<&'a ResponseHead> for ResponseBuilder {
    fn from(head: &'a ResponseHead) -> ResponseBuilder {
        // If this response has cookies, load them into a jar
        let mut jar: Option<CookieJar> = None;

        let cookies = CookieIter {
            iter: head.headers.get_all(header::SET_COOKIE),
        };
        for c in cookies {
            if let Some(ref mut j) = jar {
                j.add_original(c.into_owned());
            } else {
                let mut j = CookieJar::new();
                j.add_original(c.into_owned());
                jar = Some(j);
            }
        }

        let mut msg = BoxedResponseHead::new(head.status);
        msg.version = head.version;
        msg.reason = head.reason;
        for (k, v) in &head.headers {
            msg.headers.append(k.clone(), v.clone());
        }
        msg.no_chunking(!head.chunked());

        ResponseBuilder {
            head: Some(msg),
            err: None,
            cookies: jar,
        }
    }
}

impl IntoFuture for ResponseBuilder {
    type Item = Response;
    type Error = Error;
    type Future = FutureResult<Response, Error>;

    fn into_future(mut self) -> Self::Future {
        ok(self.finish())
    }
}

impl fmt::Debug for ResponseBuilder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let head = self.head.as_ref().unwrap();

        let res = writeln!(
            f,
            "\nResponseBuilder {:?} {}{}",
            head.version,
            head.status,
            head.reason.unwrap_or(""),
        );
        let _ = writeln!(f, "  headers:");
        for (key, val) in head.headers.iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        res
    }
}

/// Helper converters
impl<I: Into<Response>, E: Into<Error>> From<Result<I, E>> for Response {
    fn from(res: Result<I, E>) -> Self {
        match res {
            Ok(val) => val.into(),
            Err(err) => err.into().into(),
        }
    }
}

impl From<ResponseBuilder> for Response {
    fn from(mut builder: ResponseBuilder) -> Self {
        builder.finish()
    }
}

impl From<&'static str> for Response {
    fn from(val: &'static str) -> Self {
        Response::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(val)
    }
}

impl From<&'static [u8]> for Response {
    fn from(val: &'static [u8]) -> Self {
        Response::Ok()
            .content_type("application/octet-stream")
            .body(val)
    }
}

impl From<String> for Response {
    fn from(val: String) -> Self {
        Response::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(val)
    }
}

impl<'a> From<&'a String> for Response {
    fn from(val: &'a String) -> Self {
        Response::Ok()
            .content_type("text/plain; charset=utf-8")
            .body(val)
    }
}

impl From<Bytes> for Response {
    fn from(val: Bytes) -> Self {
        Response::Ok()
            .content_type("application/octet-stream")
            .body(val)
    }
}

impl From<BytesMut> for Response {
    fn from(val: BytesMut) -> Self {
        Response::Ok()
            .content_type("application/octet-stream")
            .body(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::Body;
    use crate::http::header::{HeaderValue, CONTENT_TYPE, COOKIE, SET_COOKIE};

    #[test]
    fn test_debug() {
        let resp = Response::Ok()
            .header(COOKIE, HeaderValue::from_static("cookie1=value1; "))
            .header(COOKIE, HeaderValue::from_static("cookie2=value2; "))
            .finish();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("Response"));
    }

    #[test]
    fn test_response_cookies() {
        use crate::httpmessage::HttpMessage;

        let req = crate::test::TestRequest::default()
            .header(COOKIE, "cookie1=value1")
            .header(COOKIE, "cookie2=value2")
            .finish();
        let cookies = req.cookies().unwrap();

        let resp = Response::Ok()
            .cookie(
                crate::http::Cookie::build("name", "value")
                    .domain("www.rust-lang.org")
                    .path("/test")
                    .http_only(true)
                    .max_age_time(time::Duration::days(1))
                    .finish(),
            )
            .del_cookie(&cookies[1])
            .finish();

        let mut val: Vec<_> = resp
            .headers()
            .get_all(SET_COOKIE)
            .map(|v| v.to_str().unwrap().to_owned())
            .collect();
        val.sort();
        assert!(val[0].starts_with("cookie1=; Max-Age=0;"));
        assert_eq!(
            val[1],
            "name=value; HttpOnly; Path=/test; Domain=www.rust-lang.org; Max-Age=86400"
        );
    }

    #[test]
    fn test_update_response_cookies() {
        let mut r = Response::Ok()
            .cookie(crate::http::Cookie::new("original", "val100"))
            .finish();

        r.add_cookie(&crate::http::Cookie::new("cookie2", "val200"))
            .unwrap();
        r.add_cookie(&crate::http::Cookie::new("cookie2", "val250"))
            .unwrap();
        r.add_cookie(&crate::http::Cookie::new("cookie3", "val300"))
            .unwrap();

        assert_eq!(r.cookies().count(), 4);
        r.del_cookie("cookie2");

        let mut iter = r.cookies();
        let v = iter.next().unwrap();
        assert_eq!((v.name(), v.value()), ("cookie3", "val300"));
        let v = iter.next().unwrap();
        assert_eq!((v.name(), v.value()), ("original", "val100"));
    }

    #[test]
    fn test_basic_builder() {
        let resp = Response::Ok().header("X-TEST", "value").finish();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_upgrade() {
        let resp = Response::build(StatusCode::OK)
            .upgrade("websocket")
            .finish();
        assert!(resp.upgrade());
        assert_eq!(
            resp.headers().get(header::UPGRADE).unwrap(),
            HeaderValue::from_static("websocket")
        );
    }

    #[test]
    fn test_force_close() {
        let resp = Response::build(StatusCode::OK).force_close().finish();
        assert!(!resp.keep_alive())
    }

    #[test]
    fn test_content_type() {
        let resp = Response::build(StatusCode::OK)
            .content_type("text/plain")
            .body(Body::Empty);
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/plain")
    }

    #[test]
    fn test_json() {
        let resp = Response::build(StatusCode::OK).json(vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(resp.body().get_ref(), b"[\"v1\",\"v2\",\"v3\"]");
    }

    #[test]
    fn test_json_ct() {
        let resp = Response::build(StatusCode::OK)
            .header(CONTENT_TYPE, "text/json")
            .json(vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("text/json"));
        assert_eq!(resp.body().get_ref(), b"[\"v1\",\"v2\",\"v3\"]");
    }

    #[test]
    fn test_json2() {
        let resp = Response::build(StatusCode::OK).json2(&vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("application/json"));
        assert_eq!(resp.body().get_ref(), b"[\"v1\",\"v2\",\"v3\"]");
    }

    #[test]
    fn test_json2_ct() {
        let resp = Response::build(StatusCode::OK)
            .header(CONTENT_TYPE, "text/json")
            .json2(&vec!["v1", "v2", "v3"]);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap();
        assert_eq!(ct, HeaderValue::from_static("text/json"));
        assert_eq!(resp.body().get_ref(), b"[\"v1\",\"v2\",\"v3\"]");
    }

    #[test]
    fn test_into_response() {
        let resp: Response = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let resp: Response = b"test".as_ref().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let resp: Response = "test".to_owned().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let resp: Response = (&"test".to_owned()).into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let b = Bytes::from_static(b"test");
        let resp: Response = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let b = Bytes::from_static(b"test");
        let resp: Response = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let b = BytesMut::from("test");
        let resp: Response = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");
    }

    #[test]
    fn test_into_builder() {
        let mut resp: Response = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);

        resp.add_cookie(&crate::http::Cookie::new("cookie1", "val100"))
            .unwrap();

        let mut builder: ResponseBuilder = resp.into();
        let resp = builder.status(StatusCode::BAD_REQUEST).finish();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let cookie = resp.cookies().next().unwrap();
        assert_eq!((cookie.name(), cookie.value()), ("cookie1", "val100"));
    }
}

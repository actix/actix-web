//! HTTP responses.

use std::{
    cell::{Ref, RefMut},
    fmt,
    future::Future,
    pin::Pin,
    str,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_core::Stream;

use crate::{
    body::{Body, BodyStream, MessageBody, ResponseBody},
    error::Error,
    extensions::Extensions,
    header::{IntoHeaderPair, IntoHeaderValue},
    http::{header, Error as HttpError, HeaderMap, StatusCode},
    message::{BoxedResponseHead, ConnectionType, ResponseHead},
};

/// An HTTP Response
pub struct Response<B> {
    head: BoxedResponseHead,
    body: ResponseBody<B>,
    error: Option<Error>,
}

impl Response<Body> {
    /// Create HTTP response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> ResponseBuilder {
        ResponseBuilder::new(status)
    }

    /// Create HTTP response builder
    #[inline]
    pub fn build_from<T: Into<ResponseBuilder>>(source: T) -> ResponseBuilder {
        source.into()
    }

    /// Constructs a response
    #[inline]
    pub fn new(status: StatusCode) -> Response<Body> {
        Response {
            head: BoxedResponseHead::new(status),
            body: ResponseBody::Body(Body::Empty),
            error: None,
        }
    }

    /// Constructs an error response
    #[inline]
    pub fn from_error(error: Error) -> Response<Body> {
        let mut resp = error.as_response_error().error_response();
        if resp.head.status == StatusCode::INTERNAL_SERVER_ERROR {
            error!("Internal Server Error: {:?}", error);
        }
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
    /// Mutable reference to a HTTP message part of the response
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
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.head.extensions.borrow()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        self.head.extensions.borrow_mut()
    }

    /// Get body of this response
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

/// An HTTP response builder.
///
/// This type can be used to construct an instance of `Response` through a builder-like pattern.
pub struct ResponseBuilder {
    head: Option<BoxedResponseHead>,
    err: Option<HttpError>,
}

impl ResponseBuilder {
    #[inline]
    /// Create response builder
    pub fn new(status: StatusCode) -> Self {
        ResponseBuilder {
            head: Some(BoxedResponseHead::new(status)),
            err: None,
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

    /// Insert a header, replacing any that were set with an equivalent field name.
    ///
    /// ```
    /// # use actix_http::Response;
    /// use actix_http::http::header;
    ///
    /// Response::Ok()
    ///     .insert_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .insert_header(("X-TEST", "value"))
    ///     .finish();
    /// ```
    pub fn insert_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            match header.try_into_header_pair() {
                Ok((key, value)) => {
                    parts.headers.insert(key, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }

        self
    }

    /// Append a header, keeping any that were set with an equivalent field name.
    ///
    /// ```
    /// # use actix_http::Response;
    /// use actix_http::http::header;
    ///
    /// Response::Ok()
    ///     .append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON))
    ///     .append_header(("X-TEST", "value1"))
    ///     .append_header(("X-TEST", "value2"))
    ///     .finish();
    /// ```
    pub fn append_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            match header.try_into_header_pair() {
                Ok((key, value)) => parts.headers.append(key, value),
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

        if let Ok(value) = value.try_into_value() {
            self.insert_header((header::UPGRADE, value));
        }

        self
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
    pub fn no_chunking(&mut self, len: u64) -> &mut Self {
        let mut buf = itoa::Buffer::new();
        self.insert_header((header::CONTENT_LENGTH, buf.format(len)));

        if let Some(parts) = parts(&mut self.head, &self.err) {
            parts.no_chunking(true);
        }
        self
    }

    /// Set response content type.
    #[inline]
    pub fn content_type<V>(&mut self, value: V) -> &mut Self
    where
        V: IntoHeaderValue,
    {
        if let Some(parts) = parts(&mut self.head, &self.err) {
            match value.try_into_value() {
                Ok(value) => {
                    parts.headers.insert(header::CONTENT_TYPE, value);
                }
                Err(e) => self.err = Some(e.into()),
            };
        }
        self
    }

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions.borrow()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        let head = self.head.as_ref().expect("cannot reuse response builder");
        head.extensions.borrow_mut()
    }

    /// Set a body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    #[inline]
    pub fn body<B: Into<Body>>(&mut self, body: B) -> Response<Body> {
        self.message_body(body.into())
    }

    /// Set a body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    pub fn message_body<B>(&mut self, body: B) -> Response<B> {
        if let Some(e) = self.err.take() {
            return Response::from(Error::from(e)).into_body();
        }

        let response = self.head.take().expect("cannot reuse response builder");

        Response {
            head: response,
            body: ResponseBody::Body(body),
            error: None,
        }
    }

    /// Set a streaming body and generate `Response`.
    ///
    /// `ResponseBuilder` can not be used after this call.
    #[inline]
    pub fn streaming<S, E>(&mut self, stream: S) -> Response<Body>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + 'static,
        E: Into<Error> + 'static,
    {
        self.body(Body::from_message(BodyStream::new(stream)))
    }

    /// Set an empty body and generate `Response`
    ///
    /// `ResponseBuilder` can not be used after this call.
    #[inline]
    pub fn finish(&mut self) -> Response<Body> {
        self.body(Body::Empty)
    }

    /// This method construct new `ResponseBuilder`
    pub fn take(&mut self) -> ResponseBuilder {
        ResponseBuilder {
            head: self.head.take(),
            err: self.err.take(),
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
        ResponseBuilder {
            head: Some(res.head),
            err: None,
        }
    }
}

/// Convert `ResponseHead` to a `ResponseBuilder`
impl<'a> From<&'a ResponseHead> for ResponseBuilder {
    fn from(head: &'a ResponseHead) -> ResponseBuilder {
        let mut msg = BoxedResponseHead::new(head.status);
        msg.version = head.version;
        msg.reason = head.reason;

        for (k, v) in head.headers.iter() {
            msg.headers.append(k.clone(), v.clone());
        }

        msg.no_chunking(!head.chunked());

        ResponseBuilder {
            head: Some(msg),
            err: None,
        }
    }
}

impl Future for ResponseBuilder {
    type Output = Result<Response<Body>, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(self.finish()))
    }
}

impl fmt::Debug for ResponseBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
impl<I: Into<Response<Body>>, E: Into<Error>> From<Result<I, E>> for Response<Body> {
    fn from(res: Result<I, E>) -> Self {
        match res {
            Ok(val) => val.into(),
            Err(err) => err.into().into(),
        }
    }
}

impl From<ResponseBuilder> for Response<Body> {
    fn from(mut builder: ResponseBuilder) -> Self {
        builder.finish()
    }
}

impl From<&'static str> for Response<Body> {
    fn from(val: &'static str) -> Self {
        Response::Ok()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(val)
    }
}

impl From<&'static [u8]> for Response<Body> {
    fn from(val: &'static [u8]) -> Self {
        Response::Ok()
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(val)
    }
}

impl From<String> for Response<Body> {
    fn from(val: String) -> Self {
        Response::Ok()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(val)
    }
}

impl<'a> From<&'a String> for Response<Body> {
    fn from(val: &'a String) -> Self {
        Response::Ok()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(val)
    }
}

impl From<Bytes> for Response<Body> {
    fn from(val: Bytes) -> Self {
        Response::Ok()
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(val)
    }
}

impl From<BytesMut> for Response<Body> {
    fn from(val: BytesMut) -> Self {
        Response::Ok()
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::Body;
    use crate::http::header::{HeaderName, HeaderValue, CONTENT_TYPE, COOKIE};

    #[test]
    fn test_debug() {
        let resp = Response::Ok()
            .append_header((COOKIE, HeaderValue::from_static("cookie1=value1; ")))
            .append_header((COOKIE, HeaderValue::from_static("cookie2=value2; ")))
            .finish();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("Response"));
    }

    #[test]
    fn test_basic_builder() {
        let resp = Response::Ok().insert_header(("X-TEST", "value")).finish();
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
    fn test_into_response() {
        let resp: Response<Body> = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let resp: Response<Body> = b"test".as_ref().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let resp: Response<Body> = "test".to_owned().into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let resp: Response<Body> = (&"test".to_owned()).into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let b = Bytes::from_static(b"test");
        let resp: Response<Body> = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let b = Bytes::from_static(b"test");
        let resp: Response<Body> = b.into();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.body().get_ref(), b"test");

        let b = BytesMut::from("test");
        let resp: Response<Body> = b.into();
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
        let mut resp: Response<Body> = "test".into();
        assert_eq!(resp.status(), StatusCode::OK);

        resp.headers_mut().insert(
            HeaderName::from_static("cookie"),
            HeaderValue::from_static("cookie1=val100"),
        );

        let mut builder: ResponseBuilder = resp.into();
        let resp = builder.status(StatusCode::BAD_REQUEST).finish();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let cookie = resp.headers().get_all("Cookie").next().unwrap();
        assert_eq!(cookie.to_str().unwrap(), "cookie1=val100");
    }

    #[test]
    fn response_builder_header_insert_kv() {
        let mut res = Response::Ok();
        res.insert_header(("Content-Type", "application/octet-stream"));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_insert_typed() {
        let mut res = Response::Ok();
        res.insert_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        let res = res.finish();

        assert_eq!(
            res.headers().get("Content-Type"),
            Some(&HeaderValue::from_static("application/octet-stream"))
        );
    }

    #[test]
    fn response_builder_header_append_kv() {
        let mut res = Response::Ok();
        res.append_header(("Content-Type", "application/octet-stream"));
        res.append_header(("Content-Type", "application/json"));
        let res = res.finish();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }

    #[test]
    fn response_builder_header_append_typed() {
        let mut res = Response::Ok();
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM));
        res.append_header((header::CONTENT_TYPE, mime::APPLICATION_JSON));
        let res = res.finish();

        let headers: Vec<_> = res.headers().get_all("Content-Type").cloned().collect();
        assert_eq!(headers.len(), 2);
        assert!(headers.contains(&HeaderValue::from_static("application/octet-stream")));
        assert!(headers.contains(&HeaderValue::from_static("application/json")));
    }
}

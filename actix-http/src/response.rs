//! HTTP response.

use std::{
    cell::{Ref, RefMut},
    fmt,
    future::Future,
    pin::Pin,
    str,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};

use crate::{
    body::{Body, MessageBody, ResponseBody},
    error::Error,
    extensions::Extensions,
    http::{HeaderMap, StatusCode},
    message::{BoxedResponseHead, ResponseHead},
    ResponseBuilder,
};

/// An HTTP response.
pub struct Response<B> {
    pub(crate) head: BoxedResponseHead,
    pub(crate) body: ResponseBody<B>,
    pub(crate) error: Option<Error>,
}

impl Response<Body> {
    /// Constructs a response
    #[inline]
    pub fn new(status: StatusCode) -> Response<Body> {
        Response {
            head: BoxedResponseHead::new(status),
            body: ResponseBody::Body(Body::Empty),
            error: None,
        }
    }

    /// Create HTTP response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> ResponseBuilder {
        ResponseBuilder::new(status)
    }

    // just a couple frequently used shortcuts
    // this list should not grow larger than a few

    /// Creates a new response with status 200 OK.
    #[inline]
    pub fn ok() -> Response<Body> {
        Response::new(StatusCode::OK)
    }

    /// Creates a new response with status 400 Bad Request.
    #[inline]
    pub fn bad_request() -> Response<Body> {
        Response::new(StatusCode::BAD_REQUEST)
    }

    /// Creates a new response with status 404 Not Found.
    #[inline]
    pub fn not_found() -> Response<Body> {
        Response::new(StatusCode::NOT_FOUND)
    }

    /// Creates a new response with status 500 Internal Server Error.
    #[inline]
    pub fn internal_server_error() -> Response<Body> {
        Response::new(StatusCode::INTERNAL_SERVER_ERROR)
    }

    // end shortcuts

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

impl<B: Unpin> Future for Response<B> {
    type Output = Result<Response<B>, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(Response {
            head: self.head.take(),
            body: self.body.take_body(),
            error: self.error.take(),
        }))
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
        Response::build(StatusCode::OK)
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(val)
    }
}

impl From<&'static [u8]> for Response<Body> {
    fn from(val: &'static [u8]) -> Self {
        Response::build(StatusCode::OK)
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(val)
    }
}

impl From<String> for Response<Body> {
    fn from(val: String) -> Self {
        Response::build(StatusCode::OK)
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(val)
    }
}

impl<'a> From<&'a String> for Response<Body> {
    fn from(val: &'a String) -> Self {
        Response::build(StatusCode::OK)
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .body(val)
    }
}

impl From<Bytes> for Response<Body> {
    fn from(val: Bytes) -> Self {
        Response::build(StatusCode::OK)
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(val)
    }
}

impl From<BytesMut> for Response<Body> {
    fn from(val: BytesMut) -> Self {
        Response::build(StatusCode::OK)
            .content_type(mime::APPLICATION_OCTET_STREAM)
            .body(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::Body;
    use crate::http::header::{HeaderValue, CONTENT_TYPE, COOKIE};

    #[test]
    fn test_debug() {
        let resp = Response::build(StatusCode::OK)
            .append_header((COOKIE, HeaderValue::from_static("cookie1=value1; ")))
            .append_header((COOKIE, HeaderValue::from_static("cookie2=value2; ")))
            .finish();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("Response"));
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
}

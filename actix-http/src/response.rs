//! HTTP response.

use std::{
    cell::{Ref, RefMut},
    fmt, str,
};

use bytes::{Bytes, BytesMut};

use crate::{
    body::{Body, MessageBody},
    error::Error,
    extensions::Extensions,
    http::{HeaderMap, StatusCode},
    message::{BoxedResponseHead, ResponseHead},
    ResponseBuilder,
};

/// An HTTP response.
pub struct Response<B> {
    pub(crate) head: BoxedResponseHead,
    pub(crate) body: B,
    pub(crate) error: Option<Error>,
}

impl Response<Body> {
    /// Constructs a new response with default body.
    #[inline]
    pub fn new(status: StatusCode) -> Response<Body> {
        Response {
            head: BoxedResponseHead::new(status),
            body: Body::Empty,
            error: None,
        }
    }

    /// Constructs a new response builder.
    #[inline]
    pub fn build(status: StatusCode) -> ResponseBuilder {
        ResponseBuilder::new(status)
    }

    // just a couple frequently used shortcuts
    // this list should not grow larger than a few

    /// Constructs a new response with status 200 OK.
    #[inline]
    pub fn ok() -> Response<Body> {
        Response::new(StatusCode::OK)
    }

    /// Constructs a new response with status 400 Bad Request.
    #[inline]
    pub fn bad_request() -> Response<Body> {
        Response::new(StatusCode::BAD_REQUEST)
    }

    /// Constructs a new response with status 404 Not Found.
    #[inline]
    pub fn not_found() -> Response<Body> {
        Response::new(StatusCode::NOT_FOUND)
    }

    /// Constructs a new response with status 500 Internal Server Error.
    #[inline]
    pub fn internal_server_error() -> Response<Body> {
        Response::new(StatusCode::INTERNAL_SERVER_ERROR)
    }

    // end shortcuts

    /// Constructs a new response from an error.
    #[inline]
    pub fn from_error(error: Error) -> Response<Body> {
        let mut resp = error.as_response_error().error_response();
        if resp.head.status == StatusCode::INTERNAL_SERVER_ERROR {
            debug!("Internal Server Error: {:?}", error);
        }
        resp.error = Some(error);
        resp
    }
}

impl<B> Response<B> {
    /// Constructs a new response with given body.
    #[inline]
    pub fn with_body(status: StatusCode, body: B) -> Response<B> {
        Response {
            head: BoxedResponseHead::new(status),
            body: body,
            error: None,
        }
    }

    /// Return a reference to the head of this response.
    #[inline]
    pub fn head(&self) -> &ResponseHead {
        &*self.head
    }

    /// Return a mutable reference to the head of this response.
    #[inline]
    pub fn head_mut(&mut self) -> &mut ResponseHead {
        &mut *self.head
    }

    /// Return the source `error` for this response, if one is set.
    #[inline]
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    /// Return the status code of this response.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.head.status
    }

    /// Returns a mutable reference the status code of this response.
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.head.status
    }

    /// Returns a reference to response headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    /// Returns a mutable reference to response headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head.headers
    }

    /// Returns true if connection upgrade is enabled.
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.head.upgrade()
    }

    /// Returns true if keep-alive is enabled.
    pub fn keep_alive(&self) -> bool {
        self.head.keep_alive()
    }

    /// Returns a reference to the extensions of this response.
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.head.extensions.borrow()
    }

    /// Returns a mutable reference to the extensions of this response.
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        self.head.extensions.borrow_mut()
    }

    /// Returns a reference to the body of this response.
    #[inline]
    pub fn body(&self) -> &B {
        &self.body
    }

    /// Sets new body.
    pub fn set_body<B2>(self, body: B2) -> Response<B2> {
        Response {
            head: self.head,
            body,
            error: None,
        }
    }

    /// Drops body and returns new response.
    pub fn drop_body(self) -> Response<()> {
        self.set_body(())
    }

    /// Sets new body, returning new response and previous body value.
    pub(crate) fn replace_body<B2>(self, body: B2) -> (Response<B2>, B) {
        (
            Response {
                head: self.head,
                body,
                error: self.error,
            },
            self.body,
        )
    }

    /// Returns split head and body.
    ///
    /// # Implementation Notes
    /// Due to internal performance optimisations, the first element of the returned tuple is a
    /// `Response` as well but only contains the head of the response this was called on.
    pub fn into_parts(self) -> (Response<()>, B) {
        self.replace_body(())
    }

    /// Returns new response with mapped body.
    pub fn map_body<F, B2>(mut self, f: F) -> Response<B2>
    where
        F: FnOnce(&mut ResponseHead, B) -> B2,
    {
        let body = f(&mut self.head, self.body);

        Response {
            head: self.head,
            body,
            error: self.error,
        }
    }

    /// Returns body, consuming this response.
    pub fn into_body(self) -> B {
        self.body
    }
}

impl<B> fmt::Debug for Response<B>
where
    B: MessageBody,
    B::Error: Into<Error>,
{
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

impl<B: Default> Default for Response<B> {
    #[inline]
    fn default() -> Response<B> {
        Response::with_body(StatusCode::default(), B::default())
    }
}

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

//! HTTP response.

use std::{
    cell::{Ref, RefMut},
    error::Error as StdError,
    fmt, str,
};

use bytes::{Bytes, BytesMut};
use bytestring::ByteString;

use crate::{
    body::{BoxBody, MessageBody},
    extensions::Extensions,
    header::{self, IntoHeaderValue},
    http::{HeaderMap, StatusCode},
    message::{BoxedResponseHead, ResponseHead},
    ResponseBuilder,
};

/// An HTTP response.
pub struct Response<B> {
    pub(crate) head: BoxedResponseHead,
    pub(crate) body: B,
}

impl Response<BoxBody> {
    /// Constructs a new response with default body.
    #[inline]
    pub fn new(status: StatusCode) -> Self {
        Response {
            head: BoxedResponseHead::new(status),
            body: BoxBody::new(()),
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
    pub fn ok() -> Self {
        Response::new(StatusCode::OK)
    }

    /// Constructs a new response with status 400 Bad Request.
    #[inline]
    pub fn bad_request() -> Self {
        Response::new(StatusCode::BAD_REQUEST)
    }

    /// Constructs a new response with status 404 Not Found.
    #[inline]
    pub fn not_found() -> Self {
        Response::new(StatusCode::NOT_FOUND)
    }

    /// Constructs a new response with status 500 Internal Server Error.
    #[inline]
    pub fn internal_server_error() -> Self {
        Response::new(StatusCode::INTERNAL_SERVER_ERROR)
    }

    // end shortcuts
}

impl<B> Response<B> {
    /// Constructs a new response with given body.
    #[inline]
    pub fn with_body(status: StatusCode, body: B) -> Response<B> {
        Response {
            head: BoxedResponseHead::new(status),
            body,
        }
    }

    /// Returns a reference to the head of this response.
    #[inline]
    pub fn head(&self) -> &ResponseHead {
        &*self.head
    }

    /// Returns a mutable reference to the head of this response.
    #[inline]
    pub fn head_mut(&mut self) -> &mut ResponseHead {
        &mut *self.head
    }

    /// Returns the status code of this response.
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
        }
    }

    pub fn map_into_boxed_body(self) -> Response<BoxBody>
    where
        B: MessageBody + 'static,
        B::Error: Into<Box<dyn StdError + 'static>>,
    {
        self.map_body(|_, body| BoxBody::new(body))
    }

    /// Returns body, consuming this response.
    pub fn into_body(self) -> B {
        self.body
    }
}

impl<B> fmt::Debug for Response<B>
where
    B: MessageBody,
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

// TODO: fix this impl
// impl<B, I, E> From<Result<I, E>> for Response<BoxBody>
// where
//     B: MessageBody + 'static,
//     B::Error: Into<Box<dyn StdError + 'static>>,
//     I: Into<Response<B>>,
//     E: Into<Error>,
// {
//     fn from(res: Result<I, E>) -> Self {
//         match res {
//             Ok(val) => val.into(),
//             Err(err) => err.into().into(),
//         }
//     }
// }

impl From<ResponseBuilder> for Response<BoxBody> {
    fn from(mut builder: ResponseBuilder) -> Self {
        builder.finish().map_into_boxed_body()
    }
}

impl From<std::convert::Infallible> for Response<BoxBody> {
    fn from(val: std::convert::Infallible) -> Self {
        match val {}
    }
}

impl From<&'static str> for Response<&'static str> {
    fn from(val: &'static str) -> Self {
        let mut res = Response::with_body(StatusCode::OK, val);
        let mime = mime::TEXT_PLAIN_UTF_8.try_into_value().unwrap();
        res.headers_mut().insert(header::CONTENT_TYPE, mime);
        res
    }
}

impl From<&'static [u8]> for Response<&'static [u8]> {
    fn from(val: &'static [u8]) -> Self {
        let mut res = Response::with_body(StatusCode::OK, val);
        let mime = mime::APPLICATION_OCTET_STREAM.try_into_value().unwrap();
        res.headers_mut().insert(header::CONTENT_TYPE, mime);
        res
    }
}

impl From<String> for Response<String> {
    fn from(val: String) -> Self {
        let mut res = Response::with_body(StatusCode::OK, val);
        let mime = mime::TEXT_PLAIN_UTF_8.try_into_value().unwrap();
        res.headers_mut().insert(header::CONTENT_TYPE, mime);
        res
    }
}

// TODO: was this is useful impl
// impl<'a> From<&'a String> for Response<&'a String> {
//     fn from(val: &'a String) -> Self {
//          todo!()
//     }
// }

impl From<Bytes> for Response<Bytes> {
    fn from(val: Bytes) -> Self {
        let mut res = Response::with_body(StatusCode::OK, val);
        let mime = mime::APPLICATION_OCTET_STREAM.try_into_value().unwrap();
        res.headers_mut().insert(header::CONTENT_TYPE, mime);
        res
    }
}

impl From<BytesMut> for Response<BytesMut> {
    fn from(val: BytesMut) -> Self {
        let mut res = Response::with_body(StatusCode::OK, val);
        let mime = mime::APPLICATION_OCTET_STREAM.try_into_value().unwrap();
        res.headers_mut().insert(header::CONTENT_TYPE, mime);
        res
    }
}

impl From<ByteString> for Response<ByteString> {
    fn from(val: ByteString) -> Self {
        let mut res = Response::with_body(StatusCode::OK, val);
        let mime = mime::TEXT_PLAIN_UTF_8.try_into_value().unwrap();
        res.headers_mut().insert(header::CONTENT_TYPE, mime);
        res
    }
}

// TODO: impl into Response for ByteString

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        body::to_bytes,
        http::header::{HeaderValue, CONTENT_TYPE, COOKIE},
    };

    #[test]
    fn test_debug() {
        let resp = Response::build(StatusCode::OK)
            .append_header((COOKIE, HeaderValue::from_static("cookie1=value1; ")))
            .append_header((COOKIE, HeaderValue::from_static("cookie2=value2; ")))
            .finish();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("Response"));
    }

    #[actix_rt::test]
    async fn test_into_response() {
        let res = Response::from("test");
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"test"[..]);

        let res = Response::from(b"test".as_ref());
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"test"[..]);

        let res = Response::from("test".to_owned());
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"test"[..]);

        let res = Response::from("test".to_owned());
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/plain; charset=utf-8")
        );
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"test"[..]);

        let b = Bytes::from_static(b"test");
        let res = Response::from(b);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"test"[..]);

        let b = Bytes::from_static(b"test");
        let res = Response::from(b);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"test"[..]);

        let b = BytesMut::from("test");
        let res = Response::from(b);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/octet-stream")
        );

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(to_bytes(res.into_body()).await.unwrap(), &b"test"[..]);
    }
}

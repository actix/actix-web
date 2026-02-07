use std::{
    cell::{Ref, RefMut},
    fmt,
};

use actix_http::{
    body::{BoxBody, EitherBody, MessageBody},
    header::HeaderMap,
    Extensions, Response, ResponseHead, StatusCode,
};
#[cfg(feature = "cookies")]
use {
    actix_http::{
        error::HttpError,
        header::{self, HeaderValue},
    },
    cookie::Cookie,
};

use crate::{error::Error, HttpRequest, HttpResponseBuilder, Responder};

/// An outgoing response.
pub struct HttpResponse<B = BoxBody> {
    res: Response<B>,
    error: Option<Error>,
}

impl HttpResponse<BoxBody> {
    /// Constructs a response.
    #[inline]
    pub fn new(status: StatusCode) -> Self {
        Self {
            res: Response::new(status),
            error: None,
        }
    }

    /// Constructs a response builder with specific HTTP status.
    #[inline]
    pub fn build(status: StatusCode) -> HttpResponseBuilder {
        HttpResponseBuilder::new(status)
    }

    /// Create an error response.
    #[inline]
    pub fn from_error(error: impl Into<Error>) -> Self {
        let error = error.into();
        let mut response = error.as_response_error().error_response();
        response.error = Some(error);
        response
    }
}

impl<B> HttpResponse<B> {
    /// Constructs a response with body
    #[inline]
    pub fn with_body(status: StatusCode, body: B) -> Self {
        Self {
            res: Response::with_body(status, body),
            error: None,
        }
    }

    /// Returns a reference to response head.
    #[inline]
    pub fn head(&self) -> &ResponseHead {
        self.res.head()
    }

    /// Returns a mutable reference to response head.
    #[inline]
    pub fn head_mut(&mut self) -> &mut ResponseHead {
        self.res.head_mut()
    }

    /// The source `error` for this response
    #[inline]
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    /// Get the response status code
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.res.status()
    }

    /// Set the `StatusCode` for this response
    #[inline]
    pub fn status_mut(&mut self) -> &mut StatusCode {
        self.res.status_mut()
    }

    /// Get the headers from the response
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        self.res.headers()
    }

    /// Get a mutable reference to the headers
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.res.headers_mut()
    }

    /// Get an iterator for the cookies set by this response.
    #[cfg(feature = "cookies")]
    pub fn cookies(&self) -> CookieIter<'_> {
        CookieIter {
            iter: self.headers().get_all(header::SET_COOKIE),
        }
    }

    /// Add a cookie to this response.
    ///
    /// # Errors
    /// Returns an error if the cookie results in a malformed `Set-Cookie` header.
    #[cfg(feature = "cookies")]
    pub fn add_cookie(&mut self, cookie: &Cookie<'_>) -> Result<(), HttpError> {
        HeaderValue::from_str(&cookie.to_string())
            .map(|cookie| self.headers_mut().append(header::SET_COOKIE, cookie))
            .map_err(Into::into)
    }

    /// Add a "removal" cookie to the response that matches attributes of given cookie.
    ///
    /// This will cause browsers/clients to remove stored cookies with this name.
    ///
    /// The `Set-Cookie` header added to the response will have:
    /// - name matching given cookie;
    /// - domain matching given cookie;
    /// - path matching given cookie;
    /// - an empty value;
    /// - a max-age of `0`;
    /// - an expiration date far in the past.
    ///
    /// If the cookie you're trying to remove has an explicit path or domain set, those attributes
    /// will need to be included in the cookie passed in here.
    ///
    /// # Errors
    /// Returns an error if the given name results in a malformed `Set-Cookie` header.
    #[cfg(feature = "cookies")]
    pub fn add_removal_cookie(&mut self, cookie: &Cookie<'_>) -> Result<(), HttpError> {
        let mut removal_cookie = cookie.to_owned();
        removal_cookie.make_removal();

        HeaderValue::from_str(&removal_cookie.to_string())
            .map(|cookie| self.headers_mut().append(header::SET_COOKIE, cookie))
            .map_err(Into::into)
    }

    /// Remove all cookies with the given name from this response.
    ///
    /// Returns the number of cookies removed.
    ///
    /// This method can _not_ cause a browser/client to delete any of its stored cookies. Its only
    /// purpose is to delete cookies that were added to this response using [`add_cookie`]
    /// and [`add_removal_cookie`]. Use [`add_removal_cookie`] to send a "removal" cookie.
    ///
    /// [`add_cookie`]: Self::add_cookie
    /// [`add_removal_cookie`]: Self::add_removal_cookie
    #[cfg(feature = "cookies")]
    pub fn del_cookie(&mut self, name: &str) -> usize {
        let headers = self.headers_mut();

        let vals: Vec<HeaderValue> = headers
            .get_all(header::SET_COOKIE)
            .map(|v| v.to_owned())
            .collect();

        headers.remove(header::SET_COOKIE);

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

            // put set-cookie header head back if it does not validate
            headers.append(header::SET_COOKIE, v);
        }

        count
    }

    /// Connection upgrade status
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.res.upgrade()
    }

    /// Keep-alive status for this connection
    pub fn keep_alive(&self) -> bool {
        self.res.keep_alive()
    }

    /// Returns reference to the response-local data/extensions container.
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.res.extensions()
    }

    /// Returns reference to the response-local data/extensions container.
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        self.res.extensions_mut()
    }

    /// Returns a reference to this response's body.
    #[inline]
    pub fn body(&self) -> &B {
        self.res.body()
    }

    /// Sets new body.
    pub fn set_body<B2>(self, body: B2) -> HttpResponse<B2> {
        HttpResponse {
            res: self.res.set_body(body),
            error: self.error,
        }
    }

    /// Returns split head and body.
    ///
    /// # Implementation Notes
    /// Due to internal performance optimizations, the first element of the returned tuple is an
    /// `HttpResponse` as well but only contains the head of the response this was called on.
    pub fn into_parts(self) -> (HttpResponse<()>, B) {
        let (head, body) = self.res.into_parts();

        (
            HttpResponse {
                res: head,
                error: self.error,
            },
            body,
        )
    }

    /// Drops body and returns new response.
    pub fn drop_body(self) -> HttpResponse<()> {
        HttpResponse {
            res: self.res.drop_body(),
            error: self.error,
        }
    }

    /// Map the current body type to another using a closure, returning a new response.
    ///
    /// Closure receives the response head and the current body type.
    pub fn map_body<F, B2>(self, f: F) -> HttpResponse<B2>
    where
        F: FnOnce(&mut ResponseHead, B) -> B2,
    {
        HttpResponse {
            res: self.res.map_body(f),
            error: self.error,
        }
    }

    /// Map the current body type `B` to `EitherBody::Left(B)`.
    ///
    /// Useful for middleware which can generate their own responses.
    #[inline]
    pub fn map_into_left_body<R>(self) -> HttpResponse<EitherBody<B, R>> {
        self.map_body(|_, body| EitherBody::left(body))
    }

    /// Map the current body type `B` to `EitherBody::Right(B)`.
    ///
    /// Useful for middleware which can generate their own responses.
    #[inline]
    pub fn map_into_right_body<L>(self) -> HttpResponse<EitherBody<L, B>> {
        self.map_body(|_, body| EitherBody::right(body))
    }

    /// Map the current body to a type-erased `BoxBody`.
    #[inline]
    pub fn map_into_boxed_body(self) -> HttpResponse<BoxBody>
    where
        B: MessageBody + 'static,
    {
        self.map_body(|_, body| body.boxed())
    }

    /// Returns the response body, dropping all other parts.
    pub fn into_body(self) -> B {
        self.res.into_body()
    }
}

impl<B> fmt::Debug for HttpResponse<B>
where
    B: MessageBody,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("error", &self.error)
            .field("res", &self.res)
            .finish()
    }
}

impl<B> From<Response<B>> for HttpResponse<B> {
    fn from(res: Response<B>) -> Self {
        HttpResponse { res, error: None }
    }
}

impl From<Error> for HttpResponse {
    fn from(err: Error) -> Self {
        HttpResponse::from_error(err)
    }
}

impl<B> From<HttpResponse<B>> for Response<B> {
    fn from(res: HttpResponse<B>) -> Self {
        // this impl will always be called as part of dispatcher
        res.res
    }
}

// Rationale for cfg(test): this impl causes false positives on a clippy lint (async_yields_async)
// when returning an HttpResponse from an async function/closure and it's not very useful outside of
// tests anyway.
#[cfg(test)]
mod response_fut_impl {
    use std::{
        future::Future,
        mem,
        pin::Pin,
        task::{Context, Poll},
    };

    use super::*;

    // Future is only implemented for BoxBody payload type because it's the most useful for making
    // simple handlers without async blocks. Making it generic over all MessageBody types requires a
    // future impl on Response which would cause its body field to be, undesirably, Option<B>.
    //
    // This impl is not particularly efficient due to the Response construction and should probably
    // not be invoked if performance is important. Prefer an async fn/block in such cases.
    impl Future for HttpResponse<BoxBody> {
        type Output = Result<Response<BoxBody>, Error>;

        fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            if let Some(err) = self.error.take() {
                return Poll::Ready(Err(err));
            }

            Poll::Ready(Ok(mem::replace(
                &mut self.res,
                Response::new(StatusCode::default()),
            )))
        }
    }
}

impl<B> Responder for HttpResponse<B>
where
    B: MessageBody + 'static,
{
    type Body = B;

    #[inline]
    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        self
    }
}

#[cfg(feature = "cookies")]
pub struct CookieIter<'a> {
    iter: std::slice::Iter<'a, HeaderValue>,
}

#[cfg(feature = "cookies")]
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

#[cfg(test)]
mod tests {
    use static_assertions::assert_impl_all;

    use super::*;
    use crate::http::header::COOKIE;

    assert_impl_all!(HttpResponse: Responder);
    assert_impl_all!(HttpResponse<String>: Responder);
    assert_impl_all!(HttpResponse<&'static str>: Responder);
    assert_impl_all!(HttpResponse<crate::body::None>: Responder);

    #[test]
    fn test_debug() {
        let resp = HttpResponse::Ok()
            .append_header((COOKIE, HeaderValue::from_static("cookie1=value1; ")))
            .append_header((COOKIE, HeaderValue::from_static("cookie2=value2; ")))
            .finish();
        let dbg = format!("{:?}", resp);
        assert!(dbg.contains("HttpResponse"));
    }
}

#[cfg(test)]
#[cfg(feature = "cookies")]
mod cookie_tests {
    use super::*;

    #[test]
    fn removal_cookies() {
        let mut res = HttpResponse::Ok().finish();
        let cookie = Cookie::new("foo", "");
        res.add_removal_cookie(&cookie).unwrap();
        let set_cookie_hdr = res.headers().get(header::SET_COOKIE).unwrap();
        assert_eq!(
            &set_cookie_hdr.as_bytes()[..25],
            &b"foo=; Max-Age=0; Expires="[..],
            "unexpected set-cookie value: {:?}",
            set_cookie_hdr.to_str()
        );
    }
}

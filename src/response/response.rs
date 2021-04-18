use std::{
    cell::{Ref, RefMut},
    fmt,
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{
    body::{Body, MessageBody, ResponseBody},
    http::{header::HeaderMap, StatusCode},
    Extensions, Response, ResponseHead,
};

#[cfg(feature = "cookies")]
use {
    actix_http::http::{
        header::{self, HeaderValue},
        Error as HttpError,
    },
    cookie::Cookie,
};

use crate::{error::Error, HttpResponseBuilder};

/// An HTTP Response
pub struct HttpResponse<B = Body> {
    res: Response<B>,
    error: Option<Error>,
}

impl HttpResponse<Body> {
    /// Create HTTP response builder with specific status.
    #[inline]
    pub fn build(status: StatusCode) -> HttpResponseBuilder {
        HttpResponseBuilder::new(status)
    }

    /// Create a response.
    #[inline]
    pub fn new(status: StatusCode) -> Self {
        Self {
            res: Response::new(status),
            error: None,
        }
    }

    /// Create an error response.
    #[inline]
    pub fn from_error(error: Error) -> Self {
        let res = error.as_response_error().error_response();

        Self {
            res,
            error: Some(error),
        }
    }

    /// Convert response to response with body
    pub fn into_body<B>(self) -> HttpResponse<B> {
        HttpResponse {
            res: self.res.into_body(),
            error: self.error,
        }
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

    /// Add a cookie to this response
    #[cfg(feature = "cookies")]
    pub fn add_cookie(&mut self, cookie: &Cookie<'_>) -> Result<(), HttpError> {
        HeaderValue::from_str(&cookie.to_string())
            .map(|c| {
                self.headers_mut().append(header::SET_COOKIE, c);
            })
            .map_err(|e| e.into())
    }

    /// Remove all cookies with the given name from this response. Returns
    /// the number of cookies removed.
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

    /// Responses extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.res.extensions()
    }

    /// Mutable reference to a the response's extensions
    #[inline]
    pub fn extensions_mut(&mut self) -> RefMut<'_, Extensions> {
        self.res.extensions_mut()
    }

    /// Get body of this response
    #[inline]
    pub fn body(&self) -> &ResponseBody<B> {
        self.res.body()
    }

    /// Set a body
    pub fn set_body<B2>(self, body: B2) -> HttpResponse<B2> {
        HttpResponse {
            res: self.res.set_body(body),
            error: None,
            // error: self.error, ??
        }
    }

    /// Split response and body
    pub fn into_parts(self) -> (HttpResponse<()>, ResponseBody<B>) {
        let (head, body) = self.res.into_parts();

        (
            HttpResponse {
                res: head,
                error: None,
            },
            body,
        )
    }

    /// Drop request's body
    pub fn drop_body(self) -> HttpResponse<()> {
        HttpResponse {
            res: self.res.drop_body(),
            error: None,
        }
    }

    /// Set a body and return previous body value
    pub fn map_body<F, B2>(self, f: F) -> HttpResponse<B2>
    where
        F: FnOnce(&mut ResponseHead, ResponseBody<B>) -> ResponseBody<B2>,
    {
        HttpResponse {
            res: self.res.map_body(f),
            error: self.error,
        }
    }

    /// Extract response body
    pub fn take_body(&mut self) -> ResponseBody<B> {
        self.res.take_body()
    }
}

impl<B: MessageBody> fmt::Debug for HttpResponse<B> {
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

        // TODO: expose cause somewhere?
        // if let Some(err) = res.error {
        //     eprintln!("impl<B> From<HttpResponse<B>> for Response<B> let Some(err)");
        //     return Response::from_error(err).into_body();
        // }

        res.res
    }
}

impl Future for HttpResponse {
    type Output = Result<Response<Body>, Error>;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(err) = self.error.take() {
            return Poll::Ready(Ok(Response::from_error(err).into_body()));
        }

        Poll::Ready(Ok(mem::replace(
            &mut self.res,
            Response::new(StatusCode::default()),
        )))
    }
}

#[cfg(feature = "cookies")]
pub struct CookieIter<'a> {
    iter: header::GetAll<'a>,
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
    use super::*;
    use crate::http::header::{HeaderValue, COOKIE};

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

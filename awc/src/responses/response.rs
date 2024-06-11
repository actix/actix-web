use std::{
    cell::{Ref, RefCell, RefMut},
    fmt, mem,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use actix_http::{
    error::PayloadError, header::HeaderMap, BoxedPayloadStream, Extensions, HttpMessage, Payload,
    ResponseHead, StatusCode, Version,
};
use actix_rt::time::{sleep, Sleep};
use bytes::Bytes;
use futures_core::Stream;
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;

use super::{JsonBody, ResponseBody, ResponseTimeout};
#[cfg(feature = "cookies")]
use crate::cookie::{Cookie, ParseError as CookieParseError};

pin_project! {
    /// Client Response
    pub struct ClientResponse<S = BoxedPayloadStream> {
        pub(crate) head: ResponseHead,
        #[pin]
        pub(crate) payload: Payload<S>,
        pub(crate) timeout: ResponseTimeout,
        pub(crate) extensions: RefCell<Extensions>,

    }
}

impl<S> ClientResponse<S> {
    /// Create new Request instance
    pub(crate) fn new(head: ResponseHead, payload: Payload<S>) -> Self {
        ClientResponse {
            head,
            payload,
            timeout: ResponseTimeout::default(),
            extensions: RefCell::new(Extensions::new()),
        }
    }

    #[inline]
    pub(crate) fn head(&self) -> &ResponseHead {
        &self.head
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.head().status
    }

    #[inline]
    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Map the current body type to another using a closure. Returns a new response.
    ///
    /// Closure receives the response head and the current body type.
    pub fn map_body<F, U>(mut self, f: F) -> ClientResponse<U>
    where
        F: FnOnce(&mut ResponseHead, Payload<S>) -> Payload<U>,
    {
        let payload = f(&mut self.head, self.payload);

        ClientResponse {
            payload,
            head: self.head,
            timeout: self.timeout,
            extensions: self.extensions,
        }
    }

    /// Set a timeout duration for [`ClientResponse`](self::ClientResponse).
    ///
    /// This duration covers the duration of processing the response body stream
    /// and would end it as timeout error when deadline met.
    ///
    /// Disabled by default.
    pub fn timeout(self, dur: Duration) -> Self {
        let timeout = match self.timeout {
            ResponseTimeout::Disabled(Some(mut timeout))
            | ResponseTimeout::Enabled(mut timeout) => match Instant::now().checked_add(dur) {
                Some(deadline) => {
                    timeout.as_mut().reset(deadline.into());
                    ResponseTimeout::Enabled(timeout)
                }
                None => ResponseTimeout::Enabled(Box::pin(sleep(dur))),
            },
            _ => ResponseTimeout::Enabled(Box::pin(sleep(dur))),
        };

        Self {
            payload: self.payload,
            head: self.head,
            timeout,
            extensions: self.extensions,
        }
    }

    /// This method does not enable timeout. It's used to pass the boxed `Sleep` from
    /// `SendClientRequest` and reuse it's heap allocation together with it's slot in
    /// timer wheel.
    pub(crate) fn _timeout(mut self, timeout: Option<Pin<Box<Sleep>>>) -> Self {
        self.timeout = ResponseTimeout::Disabled(timeout);
        self
    }

    /// Load request cookies.
    #[cfg(feature = "cookies")]
    pub fn cookies(&self) -> Result<Ref<'_, Vec<Cookie<'static>>>, CookieParseError> {
        struct Cookies(Vec<Cookie<'static>>);

        if self.extensions().get::<Cookies>().is_none() {
            let mut cookies = Vec::new();
            for hdr in self.headers().get_all(&actix_http::header::SET_COOKIE) {
                let s = std::str::from_utf8(hdr.as_bytes()).map_err(CookieParseError::from)?;
                cookies.push(Cookie::parse_encoded(s)?.into_owned());
            }
            self.extensions_mut().insert(Cookies(cookies));
        }

        Ok(Ref::map(self.extensions(), |ext| {
            &ext.get::<Cookies>().unwrap().0
        }))
    }

    /// Return request cookie.
    #[cfg(feature = "cookies")]
    pub fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        if let Ok(cookies) = self.cookies() {
            for cookie in cookies.iter() {
                if cookie.name() == name {
                    return Some(cookie.to_owned());
                }
            }
        }
        None
    }
}

impl<S> ClientResponse<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    /// Returns a [`Future`] that consumes the body stream and resolves to [`Bytes`].
    ///
    /// # Errors
    /// `Future` implementation returns error if:
    /// - content length is greater than [limit](ResponseBody::limit) (default: 2 MiB)
    ///
    /// # Examples
    /// ```no_run
    /// # use awc::Client;
    /// # use bytes::Bytes;
    /// # #[actix_rt::main]
    /// # async fn async_ctx() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::default();
    /// let mut res = client.get("https://httpbin.org/robots.txt").send().await?;
    /// let body: Bytes = res.body().await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`Future`]: std::future::Future
    pub fn body(&mut self) -> ResponseBody<S> {
        ResponseBody::new(self)
    }

    /// Returns a [`Future`] consumes the body stream, parses JSON, and resolves to a deserialized
    /// `T` value.
    ///
    /// # Errors
    /// Future returns error if:
    /// - content type is not `application/json`;
    /// - content length is greater than [limit](JsonBody::limit) (default: 2 MiB).
    ///
    /// # Examples
    /// ```no_run
    /// # use awc::Client;
    /// # #[actix_rt::main]
    /// # async fn async_ctx() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::default();
    /// let mut res = client.get("https://httpbin.org/json").send().await?;
    /// let val = res.json::<serde_json::Value>().await?;
    /// assert!(val.is_object());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`Future`]: std::future::Future
    pub fn json<T: DeserializeOwned>(&mut self) -> JsonBody<S, T> {
        JsonBody::new(self)
    }
}

impl<S> fmt::Debug for ClientResponse<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\nClientResponse {:?} {}", self.version(), self.status(),)?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

impl<S> HttpMessage for ClientResponse<S> {
    type Stream = S;

    fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    fn take_payload(&mut self) -> Payload<S> {
        mem::replace(&mut self.payload, Payload::None)
    }

    fn extensions(&self) -> Ref<'_, Extensions> {
        self.extensions.borrow()
    }

    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions.borrow_mut()
    }
}

impl<S> Stream for ClientResponse<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.timeout.poll_timeout(cx)?;
        this.payload.poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::assert_impl_all;

    use super::*;
    use crate::any_body::AnyBody;

    assert_impl_all!(ClientResponse: Unpin);
    assert_impl_all!(ClientResponse<()>: Unpin);
    assert_impl_all!(ClientResponse<AnyBody>: Unpin);
}

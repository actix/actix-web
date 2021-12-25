use std::{
    cell::{Ref, RefMut},
    fmt,
    future::Future,
    io, mem,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use actix_http::{
    error::PayloadError, header, header::HeaderMap, BoxedPayloadStream, Extensions,
    HttpMessage, Payload, ResponseHead, StatusCode, Version,
};
use actix_rt::time::{sleep, Sleep};
use bytes::Bytes;
use futures_core::Stream;
use serde::de::DeserializeOwned;

#[cfg(feature = "cookies")]
use crate::cookie::{Cookie, ParseError as CookieParseError};

mod json_body;
mod read_body;
mod response_body;

pub use self::json_body::JsonBody;
pub use self::response_body::ResponseBody;

/// Client Response
pub struct ClientResponse<S = BoxedPayloadStream> {
    pub(crate) head: ResponseHead,
    pub(crate) payload: Payload<S>,
    pub(crate) timeout: ResponseTimeout,
}

/// helper enum with reusable sleep passed from `SendClientResponse`.
/// See `ClientResponse::_timeout` for reason.
pub(crate) enum ResponseTimeout {
    Disabled(Option<Pin<Box<Sleep>>>),
    Enabled(Pin<Box<Sleep>>),
}

impl Default for ResponseTimeout {
    fn default() -> Self {
        Self::Disabled(None)
    }
}

impl ResponseTimeout {
    fn poll_timeout(&mut self, cx: &mut Context<'_>) -> Result<(), PayloadError> {
        match *self {
            Self::Enabled(ref mut timeout) => {
                if timeout.as_mut().poll(cx).is_ready() {
                    Err(PayloadError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Response Payload IO timed out",
                    )))
                } else {
                    Ok(())
                }
            }
            Self::Disabled(_) => Ok(()),
        }
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
        self.head.extensions()
    }

    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.head.extensions_mut()
    }
}

impl<S> ClientResponse<S> {
    /// Create new Request instance
    pub(crate) fn new(head: ResponseHead, payload: Payload<S>) -> Self {
        ClientResponse {
            head,
            payload,
            timeout: ResponseTimeout::default(),
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

    /// Set a body and return previous body value
    pub fn map_body<F, U>(mut self, f: F) -> ClientResponse<U>
    where
        F: FnOnce(&mut ResponseHead, Payload<S>) -> Payload<U>,
    {
        let payload = f(&mut self.head, self.payload);

        ClientResponse {
            payload,
            head: self.head,
            timeout: self.timeout,
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
            for hdr in self.headers().get_all(&header::SET_COOKIE) {
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
    /// Loads HTTP response's body.
    /// Consumes body stream and parses JSON, resolving to a deserialized `T` value.
    ///
    /// # Errors
    /// `Future` implementation returns error if:
    /// - content type is not `application/json`
    /// - content length is greater than [limit](JsonBody::limit) (default: 2 MiB)
    pub fn body(&mut self) -> ResponseBody<S> {
        ResponseBody::new(self)
    }

    /// Consumes body stream and parses JSON, resolving to a deserialized `T` value.
    ///
    /// # Errors
    /// Future returns error if:
    /// - content type is not `application/json`;
    /// - content length is greater than [limit](JsonBody::limit) (default: 2 MiB).
    pub fn json<T: DeserializeOwned>(&mut self) -> JsonBody<S, T> {
        JsonBody::new(self)
    }
}

impl<S> Stream for ClientResponse<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.timeout.poll_timeout(cx)?;

        Pin::new(&mut this.payload).poll_next(cx)
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

/// Default body size limit: 2MiB
const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use static_assertions::assert_impl_all;

    use super::*;

    assert_impl_all!(ClientResponse: Unpin);
}

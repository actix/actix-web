use std::cell::{Ref, RefMut};
use std::fmt;

use bytes::{Bytes, BytesMut};
use futures::{Future, Poll, Stream};

use actix_http::error::PayloadError;
use actix_http::http::header::{CONTENT_LENGTH, SET_COOKIE};
use actix_http::http::{HeaderMap, StatusCode, Version};
use actix_http::{Extensions, HttpMessage, Payload, PayloadStream, ResponseHead};

use actix_http::cookie::Cookie;
use actix_http::error::CookieParseError;

/// Client Response
pub struct ClientResponse<S = PayloadStream> {
    pub(crate) head: ResponseHead,
    pub(crate) payload: Payload<S>,
}

impl<S> HttpMessage for ClientResponse<S> {
    type Stream = S;

    fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    fn extensions(&self) -> Ref<Extensions> {
        self.head.extensions()
    }

    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.head.extensions_mut()
    }

    fn take_payload(&mut self) -> Payload<S> {
        std::mem::replace(&mut self.payload, Payload::None)
    }

    /// Load request cookies.
    #[inline]
    fn cookies(&self) -> Result<Ref<Vec<Cookie<'static>>>, CookieParseError> {
        struct Cookies(Vec<Cookie<'static>>);

        if self.extensions().get::<Cookies>().is_none() {
            let mut cookies = Vec::new();
            for hdr in self.headers().get_all(SET_COOKIE) {
                let s = std::str::from_utf8(hdr.as_bytes())
                    .map_err(CookieParseError::from)?;
                cookies.push(Cookie::parse_encoded(s)?.into_owned());
            }
            self.extensions_mut().insert(Cookies(cookies));
        }
        Ok(Ref::map(self.extensions(), |ext| {
            &ext.get::<Cookies>().unwrap().0
        }))
    }
}

impl<S> ClientResponse<S> {
    /// Create new Request instance
    pub(crate) fn new(head: ResponseHead, payload: Payload<S>) -> Self {
        ClientResponse { head, payload }
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
    /// Returns Request's headers.
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
        }
    }
}

impl<S> ClientResponse<S>
where
    S: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    /// Load http response's body.
    pub fn body(&mut self) -> MessageBody<S> {
        MessageBody::new(self)
    }
}

impl<S> Stream for ClientResponse<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.payload.poll()
    }
}

impl<S> fmt::Debug for ClientResponse<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "\nClientResponse {:?} {}", self.version(), self.status(),)?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

/// Future that resolves to a complete http message body.
pub struct MessageBody<S> {
    limit: usize,
    length: Option<usize>,
    stream: Option<Payload<S>>,
    err: Option<PayloadError>,
    fut: Option<Box<Future<Item = Bytes, Error = PayloadError>>>,
}

impl<S> MessageBody<S>
where
    S: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    /// Create `MessageBody` for request.
    pub fn new(res: &mut ClientResponse<S>) -> MessageBody<S> {
        let mut len = None;
        if let Some(l) = res.headers().get(CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                } else {
                    return Self::err(PayloadError::UnknownLength);
                }
            } else {
                return Self::err(PayloadError::UnknownLength);
            }
        }

        MessageBody {
            limit: 262_144,
            length: len,
            stream: Some(res.take_payload()),
            fut: None,
            err: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    fn err(e: PayloadError) -> Self {
        MessageBody {
            stream: None,
            limit: 262_144,
            fut: None,
            err: Some(e),
            length: None,
        }
    }
}

impl<S> Future for MessageBody<S>
where
    S: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        if let Some(len) = self.length.take() {
            if len > self.limit {
                return Err(PayloadError::Overflow);
            }
        }

        // future
        let limit = self.limit;
        self.fut = Some(Box::new(
            self.stream
                .take()
                .expect("Can not be used second time")
                .from_err()
                .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(PayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .map(|body| body.freeze()),
        ));
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::Async;

    use crate::{http::header, test::TestResponse};

    #[test]
    fn test_body() {
        let mut req = TestResponse::with_header(header::CONTENT_LENGTH, "xxxx").finish();
        match req.body().poll().err().unwrap() {
            PayloadError::UnknownLength => (),
            _ => unreachable!("error"),
        }

        let mut req =
            TestResponse::with_header(header::CONTENT_LENGTH, "1000000").finish();
        match req.body().poll().err().unwrap() {
            PayloadError::Overflow => (),
            _ => unreachable!("error"),
        }

        let mut req = TestResponse::default()
            .set_payload(Bytes::from_static(b"test"))
            .finish();
        match req.body().poll().ok().unwrap() {
            Async::Ready(bytes) => assert_eq!(bytes, Bytes::from_static(b"test")),
            _ => unreachable!("error"),
        }

        let mut req = TestResponse::default()
            .set_payload(Bytes::from_static(b"11111111111111"))
            .finish();
        match req.body().limit(5).poll().err().unwrap() {
            PayloadError::Overflow => (),
            _ => unreachable!("error"),
        }
    }
}

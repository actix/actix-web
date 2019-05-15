use std::cell::{Ref, RefMut};
use std::fmt;
use std::marker::PhantomData;

use bytes::{Bytes, BytesMut};
use futures::{Async, Future, Poll, Stream};

use actix_http::cookie::Cookie;
use actix_http::error::{CookieParseError, PayloadError};
use actix_http::http::header::{CONTENT_LENGTH, SET_COOKIE};
use actix_http::http::{HeaderMap, StatusCode, Version};
use actix_http::{Extensions, HttpMessage, Payload, PayloadStream, ResponseHead};
use serde::de::DeserializeOwned;

use crate::error::JsonPayloadError;

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
            for hdr in self.headers().get_all(&SET_COOKIE) {
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
        }
    }
}

impl<S> ClientResponse<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    /// Loads http response's body.
    pub fn body(&mut self) -> MessageBody<S> {
        MessageBody::new(self)
    }

    /// Loads and parse `application/json` encoded body.
    /// Return `JsonBody<T>` future. It resolves to a `T` value.
    ///
    /// Returns error:
    ///
    /// * content type is not `application/json`
    /// * content length is greater than 256k
    pub fn json<T: DeserializeOwned>(&mut self) -> JsonBody<S, T> {
        JsonBody::new(self)
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
    length: Option<usize>,
    err: Option<PayloadError>,
    fut: Option<ReadBody<S>>,
}

impl<S> MessageBody<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    /// Create `MessageBody` for request.
    pub fn new(res: &mut ClientResponse<S>) -> MessageBody<S> {
        let mut len = None;
        if let Some(l) = res.headers().get(&CONTENT_LENGTH) {
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
            length: len,
            err: None,
            fut: Some(ReadBody::new(res.take_payload(), 262_144)),
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(ref mut fut) = self.fut {
            fut.limit = limit;
        }
        self
    }

    fn err(e: PayloadError) -> Self {
        MessageBody {
            fut: None,
            err: Some(e),
            length: None,
        }
    }
}

impl<S> Future for MessageBody<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(err) = self.err.take() {
            return Err(err);
        }

        if let Some(len) = self.length.take() {
            if len > self.fut.as_ref().unwrap().limit {
                return Err(PayloadError::Overflow);
            }
        }

        self.fut.as_mut().unwrap().poll()
    }
}

/// Response's payload json parser, it resolves to a deserialized `T` value.
///
/// Returns error:
///
/// * content type is not `application/json`
/// * content length is greater than 64k
pub struct JsonBody<S, U> {
    length: Option<usize>,
    err: Option<JsonPayloadError>,
    fut: Option<ReadBody<S>>,
    _t: PhantomData<U>,
}

impl<S, U> JsonBody<S, U>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
    U: DeserializeOwned,
{
    /// Create `JsonBody` for request.
    pub fn new(req: &mut ClientResponse<S>) -> Self {
        // check content-type
        let json = if let Ok(Some(mime)) = req.mime_type() {
            mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON)
        } else {
            false
        };
        if !json {
            return JsonBody {
                length: None,
                fut: None,
                err: Some(JsonPayloadError::ContentType),
                _t: PhantomData,
            };
        }

        let mut len = None;
        if let Some(l) = req.headers().get(&CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                }
            }
        }

        JsonBody {
            length: len,
            err: None,
            fut: Some(ReadBody::new(req.take_payload(), 65536)),
            _t: PhantomData,
        }
    }

    /// Change max size of payload. By default max size is 64Kb
    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(ref mut fut) = self.fut {
            fut.limit = limit;
        }
        self
    }
}

impl<T, U> Future for JsonBody<T, U>
where
    T: Stream<Item = Bytes, Error = PayloadError>,
    U: DeserializeOwned,
{
    type Item = U;
    type Error = JsonPayloadError;

    fn poll(&mut self) -> Poll<U, JsonPayloadError> {
        if let Some(err) = self.err.take() {
            return Err(err);
        }

        if let Some(len) = self.length.take() {
            if len > self.fut.as_ref().unwrap().limit {
                return Err(JsonPayloadError::Payload(PayloadError::Overflow));
            }
        }

        let body = futures::try_ready!(self.fut.as_mut().unwrap().poll());
        Ok(Async::Ready(serde_json::from_slice::<U>(&body)?))
    }
}

struct ReadBody<S> {
    stream: Payload<S>,
    buf: BytesMut,
    limit: usize,
}

impl<S> ReadBody<S> {
    fn new(stream: Payload<S>, limit: usize) -> Self {
        Self {
            stream,
            buf: BytesMut::with_capacity(std::cmp::min(limit, 32768)),
            limit,
        }
    }
}

impl<S> Future for ReadBody<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            return match self.stream.poll()? {
                Async::Ready(Some(chunk)) => {
                    if (self.buf.len() + chunk.len()) > self.limit {
                        Err(PayloadError::Overflow)
                    } else {
                        self.buf.extend_from_slice(&chunk);
                        continue;
                    }
                }
                Async::Ready(None) => Ok(Async::Ready(self.buf.take().freeze())),
                Async::NotReady => Ok(Async::NotReady),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_http_test::block_on;
    use futures::Async;
    use serde::{Deserialize, Serialize};

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

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    fn json_eq(err: JsonPayloadError, other: JsonPayloadError) -> bool {
        match err {
            JsonPayloadError::Payload(PayloadError::Overflow) => match other {
                JsonPayloadError::Payload(PayloadError::Overflow) => true,
                _ => false,
            },
            JsonPayloadError::ContentType => match other {
                JsonPayloadError::ContentType => true,
                _ => false,
            },
            _ => false,
        }
    }

    #[test]
    fn test_json_body() {
        let mut req = TestResponse::default().finish();
        let json = block_on(JsonBody::<_, MyObject>::new(&mut req));
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let mut req = TestResponse::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            )
            .finish();
        let json = block_on(JsonBody::<_, MyObject>::new(&mut req));
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let mut req = TestResponse::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10000"),
            )
            .finish();

        let json = block_on(JsonBody::<_, MyObject>::new(&mut req).limit(100));
        assert!(json_eq(
            json.err().unwrap(),
            JsonPayloadError::Payload(PayloadError::Overflow)
        ));

        let mut req = TestResponse::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            )
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .finish();

        let json = block_on(JsonBody::<_, MyObject>::new(&mut req));
        assert_eq!(
            json.ok().unwrap(),
            MyObject {
                name: "test".to_owned()
            }
        );
    }
}

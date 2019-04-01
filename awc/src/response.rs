use std::cell::{Ref, RefMut};
use std::fmt;

use bytes::{Bytes, BytesMut};
use futures::{Future, Poll, Stream};

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

/// Response's payload json parser, it resolves to a deserialized `T` value.
///
/// Returns error:
///
/// * content type is not `application/json`
/// * content length is greater than 64k
pub struct JsonBody<S, U> {
    limit: usize,
    length: Option<usize>,
    stream: Payload<S>,
    err: Option<JsonPayloadError>,
    fut: Option<Box<Future<Item = U, Error = JsonPayloadError>>>,
}

impl<S, U> JsonBody<S, U>
where
    S: Stream<Item = Bytes, Error = PayloadError> + 'static,
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
                limit: 65536,
                length: None,
                stream: Payload::None,
                fut: None,
                err: Some(JsonPayloadError::ContentType),
            };
        }

        let mut len = None;
        if let Some(l) = req.headers().get(CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                }
            }
        }

        JsonBody {
            limit: 65536,
            length: len,
            stream: req.take_payload(),
            fut: None,
            err: None,
        }
    }

    /// Change max size of payload. By default max size is 64Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl<T, U> Future for JsonBody<T, U>
where
    T: Stream<Item = Bytes, Error = PayloadError> + 'static,
    U: DeserializeOwned + 'static,
{
    type Item = U;
    type Error = JsonPayloadError;

    fn poll(&mut self) -> Poll<U, JsonPayloadError> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        let limit = self.limit;
        if let Some(len) = self.length.take() {
            if len > limit {
                return Err(JsonPayloadError::Overflow);
            }
        }

        let fut = std::mem::replace(&mut self.stream, Payload::None)
            .from_err()
            .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                if (body.len() + chunk.len()) > limit {
                    Err(JsonPayloadError::Overflow)
                } else {
                    body.extend_from_slice(&chunk);
                    Ok(body)
                }
            })
            .and_then(|body| Ok(serde_json::from_slice::<U>(&body)?));
        self.fut = Some(Box::new(fut));
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::Async;
    use serde::{Deserialize, Serialize};

    use crate::{http::header, test::block_on, test::TestResponse};

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
            JsonPayloadError::Overflow => match other {
                JsonPayloadError::Overflow => true,
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
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::Overflow));

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

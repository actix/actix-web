use std::cell::{Ref, RefMut};
use std::fmt;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use futures_core::{ready, Stream};

use actix_http::cookie::Cookie;
use actix_http::error::{CookieParseError, PayloadError};
use actix_http::http::header::{CONTENT_LENGTH, SET_COOKIE};
use actix_http::http::{HeaderMap, StatusCode, Version};
use actix_http::{Extensions, HttpMessage, Payload, PayloadStream, ResponseHead};
use actix_rt::time::{sleep, Sleep};
use serde::de::DeserializeOwned;

use crate::error::JsonPayloadError;

/// Client Response
pub struct ClientResponse<S = PayloadStream> {
    pub(crate) head: ResponseHead,
    pub(crate) payload: Payload<S>,
    pub(crate) timeout: ResponseTimeout,
}

// a helper enum for response timeout for reusing the boxed sleep.
// It's pass from `SendClientRequest`.
pub(crate) enum ResponseTimeout {
    Disabled(Option<Pin<Box<Sleep>>>),
    Enabled(Option<Pin<Box<Sleep>>>),
}

impl ResponseTimeout {
    fn poll_timeout(&mut self, cx: &mut Context<'_>) -> Result<(), io::Error> {
        match *self {
            Self::Disabled(_) => Ok(()),
            Self::Enabled(Some(ref mut timeout)) => {
                if timeout.as_mut().poll(cx).is_ready() {
                    Ok(())
                } else {
                    Err(Self::err())
                }
            }
            Self::Enabled(None) => Err(Self::err()),
        }
    }

    fn take(&mut self) -> Option<Pin<Box<Sleep>>> {
        match *self {
            Self::Disabled(_) => None,
            Self::Enabled(ref mut timeout) => timeout.take(),
        }
    }

    fn err() -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, "Response Payload IO timed out")
    }
}

impl<S> HttpMessage for ClientResponse<S> {
    type Stream = S;

    fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    fn take_payload(&mut self) -> Payload<S> {
        std::mem::replace(&mut self.payload, Payload::None)
    }

    fn extensions(&self) -> Ref<'_, Extensions> {
        self.head.extensions()
    }

    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.head.extensions_mut()
    }

    /// Load request cookies.
    #[inline]
    fn cookies(&self) -> Result<Ref<'_, Vec<Cookie<'static>>>, CookieParseError> {
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
        ClientResponse {
            head,
            payload,
            timeout: ResponseTimeout::Disabled(None),
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
            | ResponseTimeout::Enabled(Some(mut timeout)) => {
                match Instant::now().checked_add(dur) {
                    Some(deadline) => {
                        timeout.as_mut().reset(deadline.into());
                        ResponseTimeout::Enabled(Some(timeout))
                    }
                    None => ResponseTimeout::Enabled(Some(Box::pin(sleep(dur)))),
                }
            }
            _ => ResponseTimeout::Enabled(Some(Box::pin(sleep(dur)))),
        };

        Self {
            payload: self.payload,
            head: self.head,
            timeout,
        }
    }

    // this method does not enable timeout. It's used to pass the boxed Sleep from
    // `SendClientRequest` and reuse it's heap allocation together with it's slot
    // in timer wheel.
    pub(crate) fn _timeout(mut self, timeout: Option<Pin<Box<Sleep>>>) -> Self {
        self.timeout = ResponseTimeout::Disabled(timeout);
        self
    }
}

impl<S> ClientResponse<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
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
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.timeout.poll_timeout(cx).map_err(PayloadError::Io)?;

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

/// Future that resolves to a complete http message body.
pub struct MessageBody<S> {
    length: Option<usize>,
    err: Option<PayloadError>,
    timeout: Option<Pin<Box<Sleep>>>,
    fut: Option<ReadBody<S>>,
}

impl<S> MessageBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
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
            timeout: res.timeout.take(),
            fut: Some(ReadBody::new(res.take_payload(), 262_144)),
        }
    }

    /// Change max size of payload. By default max size is 256kB
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
            timeout: None,
        }
    }
}

impl<S> Future for MessageBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Output = Result<Bytes, PayloadError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if let Some(err) = this.err.take() {
            return Poll::Ready(Err(err));
        }

        if let Some(len) = this.length.take() {
            if len > this.fut.as_ref().unwrap().limit {
                return Poll::Ready(Err(PayloadError::Overflow));
            }
        }

        if let Some(ref mut timeout) = this.timeout {
            if timeout.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(PayloadError::Io(ResponseTimeout::err())));
            }
        }

        Pin::new(&mut this.fut.as_mut().unwrap()).poll(cx)
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
    timeout: Option<Pin<Box<Sleep>>>,
    fut: Option<ReadBody<S>>,
    _phantom: PhantomData<U>,
}

impl<S, U> JsonBody<S, U>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
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
                timeout: None,
                err: Some(JsonPayloadError::ContentType),
                _phantom: PhantomData,
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
            timeout: req.timeout.take(),
            fut: Some(ReadBody::new(req.take_payload(), 65536)),
            _phantom: PhantomData,
        }
    }

    /// Change max size of payload. By default max size is 64kB
    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(ref mut fut) = self.fut {
            fut.limit = limit;
        }
        self
    }
}

impl<T, U> Unpin for JsonBody<T, U>
where
    T: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
    U: DeserializeOwned,
{
}

impl<T, U> Future for JsonBody<T, U>
where
    T: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
    U: DeserializeOwned,
{
    type Output = Result<U, JsonPayloadError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(err) = self.err.take() {
            return Poll::Ready(Err(err));
        }

        if let Some(len) = self.length.take() {
            if len > self.fut.as_ref().unwrap().limit {
                return Poll::Ready(Err(JsonPayloadError::Payload(
                    PayloadError::Overflow,
                )));
            }
        }

        if let Some(ref mut timeout) = self.timeout {
            if timeout.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(JsonPayloadError::Payload(PayloadError::Io(
                    ResponseTimeout::err(),
                ))));
            }
        }

        let body = ready!(Pin::new(&mut self.get_mut().fut.as_mut().unwrap()).poll(cx))?;
        Poll::Ready(serde_json::from_slice::<U>(&body).map_err(JsonPayloadError::from))
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
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Output = Result<Bytes, PayloadError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            return match Pin::new(&mut this.stream).poll_next(cx)? {
                Poll::Ready(Some(chunk)) => {
                    if (this.buf.len() + chunk.len()) > this.limit {
                        Poll::Ready(Err(PayloadError::Overflow))
                    } else {
                        this.buf.extend_from_slice(&chunk);
                        continue;
                    }
                }
                Poll::Ready(None) => Poll::Ready(Ok(this.buf.split().freeze())),
                Poll::Pending => Poll::Pending,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    use crate::{http::header, test::TestResponse};

    #[actix_rt::test]
    async fn test_body() {
        let mut req = TestResponse::with_header(header::CONTENT_LENGTH, "xxxx").finish();
        match req.body().await.err().unwrap() {
            PayloadError::UnknownLength => {}
            _ => unreachable!("error"),
        }

        let mut req =
            TestResponse::with_header(header::CONTENT_LENGTH, "1000000").finish();
        match req.body().await.err().unwrap() {
            PayloadError::Overflow => {}
            _ => unreachable!("error"),
        }

        let mut req = TestResponse::default()
            .set_payload(Bytes::from_static(b"test"))
            .finish();
        assert_eq!(req.body().await.ok().unwrap(), Bytes::from_static(b"test"));

        let mut req = TestResponse::default()
            .set_payload(Bytes::from_static(b"11111111111111"))
            .finish();
        match req.body().limit(5).await.err().unwrap() {
            PayloadError::Overflow => {}
            _ => unreachable!("error"),
        }
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    fn json_eq(err: JsonPayloadError, other: JsonPayloadError) -> bool {
        match err {
            JsonPayloadError::Payload(PayloadError::Overflow) => {
                matches!(other, JsonPayloadError::Payload(PayloadError::Overflow))
            }
            JsonPayloadError::ContentType => {
                matches!(other, JsonPayloadError::ContentType)
            }
            _ => false,
        }
    }

    #[actix_rt::test]
    async fn test_json_body() {
        let mut req = TestResponse::default().finish();
        let json = JsonBody::<_, MyObject>::new(&mut req).await;
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let mut req = TestResponse::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            )
            .finish();
        let json = JsonBody::<_, MyObject>::new(&mut req).await;
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

        let json = JsonBody::<_, MyObject>::new(&mut req).limit(100).await;
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

        let json = JsonBody::<_, MyObject>::new(&mut req).await;
        assert_eq!(
            json.ok().unwrap(),
            MyObject {
                name: "test".to_owned()
            }
        );
    }
}

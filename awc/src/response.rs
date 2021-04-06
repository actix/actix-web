use std::{
    cell::{Ref, RefMut},
    fmt,
    future::Future,
    io,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use actix_http::{
    error::PayloadError,
    http::{header, HeaderMap, StatusCode, Version},
    Extensions, HttpMessage, Payload, PayloadStream, ResponseHead,
};
use actix_rt::time::{sleep, Sleep};
use bytes::{Bytes, BytesMut};
use futures_core::{ready, Stream};
use serde::de::DeserializeOwned;

#[cfg(feature = "cookies")]
use crate::cookie::{Cookie, ParseError as CookieParseError};
use crate::error::JsonPayloadError;

/// Client Response
pub struct ClientResponse<S = PayloadStream> {
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
        std::mem::replace(&mut self.payload, Payload::None)
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

const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Future that resolves to a complete HTTP message body.
pub struct MessageBody<S> {
    length: Option<usize>,
    timeout: ResponseTimeout,
    body: Result<ReadBody<S>, Option<PayloadError>>,
}

impl<S> MessageBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    /// Create `MessageBody` for request.
    pub fn new(res: &mut ClientResponse<S>) -> MessageBody<S> {
        let length = match res.headers().get(&header::CONTENT_LENGTH) {
            Some(value) => {
                let len = value.to_str().ok().and_then(|s| s.parse::<usize>().ok());

                match len {
                    None => return Self::err(PayloadError::UnknownLength),
                    len => len,
                }
            }
            None => None,
        };

        MessageBody {
            length,
            timeout: std::mem::take(&mut res.timeout),
            body: Ok(ReadBody::new(res.take_payload(), DEFAULT_BODY_LIMIT)),
        }
    }

    /// Change max size of payload. By default max size is 2048kB
    pub fn limit(mut self, limit: usize) -> Self {
        if let Ok(ref mut body) = self.body {
            body.limit = limit;
        }
        self
    }

    fn err(e: PayloadError) -> Self {
        MessageBody {
            length: None,
            timeout: ResponseTimeout::default(),
            body: Err(Some(e)),
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

        match this.body {
            Err(ref mut err) => Poll::Ready(Err(err.take().unwrap())),
            Ok(ref mut body) => {
                if let Some(len) = this.length.take() {
                    if len > body.limit {
                        return Poll::Ready(Err(PayloadError::Overflow));
                    }
                }

                this.timeout.poll_timeout(cx)?;

                Pin::new(body).poll(cx)
            }
        }
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
    timeout: ResponseTimeout,
    fut: Option<ReadBody<S>>,
    _phantom: PhantomData<U>,
}

impl<S, U> JsonBody<S, U>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
    U: DeserializeOwned,
{
    /// Create `JsonBody` for request.
    pub fn new(res: &mut ClientResponse<S>) -> Self {
        // check content-type
        let json = if let Ok(Some(mime)) = res.mime_type() {
            mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON)
        } else {
            false
        };
        if !json {
            return JsonBody {
                length: None,
                fut: None,
                timeout: ResponseTimeout::default(),
                err: Some(JsonPayloadError::ContentType),
                _phantom: PhantomData,
            };
        }

        let mut len = None;

        if let Some(l) = res.headers().get(&header::CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                }
            }
        }

        JsonBody {
            length: len,
            err: None,
            timeout: std::mem::take(&mut res.timeout),
            fut: Some(ReadBody::new(res.take_payload(), 65536)),
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
                return Poll::Ready(Err(JsonPayloadError::Payload(PayloadError::Overflow)));
            }
        }

        self.timeout
            .poll_timeout(cx)
            .map_err(JsonPayloadError::Payload)?;

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
            buf: BytesMut::new(),
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

        while let Some(chunk) = ready!(Pin::new(&mut this.stream).poll_next(cx)?) {
            if (this.buf.len() + chunk.len()) > this.limit {
                return Poll::Ready(Err(PayloadError::Overflow));
            }
            this.buf.extend_from_slice(&chunk);
        }

        Poll::Ready(Ok(this.buf.split().freeze()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    use crate::{http::header, test::TestResponse};

    #[actix_rt::test]
    async fn test_body() {
        let mut req = TestResponse::with_header((header::CONTENT_LENGTH, "xxxx")).finish();
        match req.body().await.err().unwrap() {
            PayloadError::UnknownLength => {}
            _ => unreachable!("error"),
        }

        let mut req = TestResponse::with_header((header::CONTENT_LENGTH, "10000000")).finish();
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
            JsonPayloadError::ContentType => matches!(other, JsonPayloadError::ContentType),
            _ => false,
        }
    }

    #[actix_rt::test]
    async fn test_json_body() {
        let mut req = TestResponse::default().finish();
        let json = JsonBody::<_, MyObject>::new(&mut req).await;
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let mut req = TestResponse::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            ))
            .finish();
        let json = JsonBody::<_, MyObject>::new(&mut req).await;
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let mut req = TestResponse::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10000"),
            ))
            .finish();

        let json = JsonBody::<_, MyObject>::new(&mut req).limit(100).await;
        assert!(json_eq(
            json.err().unwrap(),
            JsonPayloadError::Payload(PayloadError::Overflow)
        ));

        let mut req = TestResponse::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
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

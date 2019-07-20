//! Websocket integration
use std::collections::VecDeque;
use std::io;

use actix::dev::{
    AsyncContextParts, ContextFut, ContextParts, Envelope, Mailbox, StreamHandler,
    ToEnvelope,
};
use actix::fut::ActorFuture;
use actix::{
    Actor, ActorContext, ActorState, Addr, AsyncContext, Handler,
    Message as ActixMessage, SpawnHandle,
};
use actix_codec::{Decoder, Encoder};
use actix_http::ws::{hash_key, Codec};
pub use actix_http::ws::{
    CloseCode, CloseReason, Frame, HandshakeError, Message, ProtocolError,
};

use actix_web::dev::HttpResponseBuilder;
use actix_web::error::{Error, ErrorInternalServerError, PayloadError};
use actix_web::http::{header, Method, StatusCode};
use actix_web::{HttpRequest, HttpResponse};
use bytes::{Bytes, BytesMut};
use futures::sync::oneshot::Sender;
use futures::{Async, Future, Poll, Stream};

/// Do websocket handshake and start ws actor.
pub fn start<A, T>(actor: A, req: &HttpRequest, stream: T) -> Result<HttpResponse, Error>
where
    A: Actor<Context = WebsocketContext<A>> + StreamHandler<Message, ProtocolError>,
    T: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    let mut res = handshake(req)?;
    Ok(res.streaming(WebsocketContext::create(actor, stream)))
}

/// Do websocket handshake and start ws actor.
///
/// `req` is an HTTP Request that should be requesting a websocket protocol
/// change. `stream` should be a `Bytes` stream (such as
/// `actix_web::web::Payload`) that contains a stream of the body request.
///
/// If there is a problem with the handshake, an error is returned.
///
/// If successful, returns a pair where the first item is an address for the
/// created actor and the second item is the response that should be returned
/// from the websocket request.
pub fn start_with_addr<A, T>(
    actor: A,
    req: &HttpRequest,
    stream: T,
) -> Result<(Addr<A>, HttpResponse), Error>
where
    A: Actor<Context = WebsocketContext<A>> + StreamHandler<Message, ProtocolError>,
    T: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    let mut res = handshake(req)?;
    let (addr, out_stream) = WebsocketContext::create_with_addr(actor, stream);
    Ok((addr, res.streaming(out_stream)))
}

/// Do websocket handshake and start ws actor.
///
/// `protocols` is a sequence of known protocols.
pub fn start_with_protocols<A, T>(
    actor: A,
    protocols: &[&str],
    req: &HttpRequest,
    stream: T,
) -> Result<HttpResponse, Error>
where
    A: Actor<Context = WebsocketContext<A>> + StreamHandler<Message, ProtocolError>,
    T: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    let mut res = handshake_with_protocols(req, protocols)?;
    Ok(res.streaming(WebsocketContext::create(actor, stream)))
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
pub fn handshake(req: &HttpRequest) -> Result<HttpResponseBuilder, HandshakeError> {
    handshake_with_protocols(req, &[])
}

/// Prepare `WebSocket` handshake response.
///
/// This function returns handshake `HttpResponse`, ready to send to peer.
/// It does not perform any IO.
///
/// `protocols` is a sequence of known protocols. On successful handshake,
/// the returned response headers contain the first protocol in this list
/// which the server also knows.
pub fn handshake_with_protocols(
    req: &HttpRequest,
    protocols: &[&str],
) -> Result<HttpResponseBuilder, HandshakeError> {
    // WebSocket accepts only GET
    if *req.method() != Method::GET {
        return Err(HandshakeError::GetMethodRequired);
    }

    // Check for "UPGRADE" to websocket header
    let has_hdr = if let Some(hdr) = req.headers().get(&header::UPGRADE) {
        if let Ok(s) = hdr.to_str() {
            s.to_ascii_lowercase().contains("websocket")
        } else {
            false
        }
    } else {
        false
    };
    if !has_hdr {
        return Err(HandshakeError::NoWebsocketUpgrade);
    }

    // Upgrade connection
    if !req.head().upgrade() {
        return Err(HandshakeError::NoConnectionUpgrade);
    }

    // check supported version
    if !req.headers().contains_key(&header::SEC_WEBSOCKET_VERSION) {
        return Err(HandshakeError::NoVersionHeader);
    }
    let supported_ver = {
        if let Some(hdr) = req.headers().get(&header::SEC_WEBSOCKET_VERSION) {
            hdr == "13" || hdr == "8" || hdr == "7"
        } else {
            false
        }
    };
    if !supported_ver {
        return Err(HandshakeError::UnsupportedVersion);
    }

    // check client handshake for validity
    if !req.headers().contains_key(&header::SEC_WEBSOCKET_KEY) {
        return Err(HandshakeError::BadWebsocketKey);
    }
    let key = {
        let key = req.headers().get(&header::SEC_WEBSOCKET_KEY).unwrap();
        hash_key(key.as_ref())
    };

    // check requested protocols
    let protocol =
        req.headers()
            .get(&header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|req_protocols| {
                let req_protocols = req_protocols.to_str().ok()?;
                req_protocols
                    .split(", ")
                    .find(|req_p| protocols.iter().any(|p| p == req_p))
            });

    let mut response = HttpResponse::build(StatusCode::SWITCHING_PROTOCOLS)
        .upgrade("websocket")
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(header::SEC_WEBSOCKET_ACCEPT, key.as_str())
        .take();

    if let Some(protocol) = protocol {
        response.header(&header::SEC_WEBSOCKET_PROTOCOL, protocol);
    }

    Ok(response)
}

/// Execution context for `WebSockets` actors
pub struct WebsocketContext<A>
where
    A: Actor<Context = WebsocketContext<A>>,
{
    inner: ContextParts<A>,
    messages: VecDeque<Option<Message>>,
}

impl<A> ActorContext for WebsocketContext<A>
where
    A: Actor<Context = Self>,
{
    fn stop(&mut self) {
        self.inner.stop();
    }

    fn terminate(&mut self) {
        self.inner.terminate()
    }

    fn state(&self) -> ActorState {
        self.inner.state()
    }
}

impl<A> AsyncContext<A> for WebsocketContext<A>
where
    A: Actor<Context = Self>,
{
    fn spawn<F>(&mut self, fut: F) -> SpawnHandle
    where
        F: ActorFuture<Item = (), Error = (), Actor = A> + 'static,
    {
        self.inner.spawn(fut)
    }

    fn wait<F>(&mut self, fut: F)
    where
        F: ActorFuture<Item = (), Error = (), Actor = A> + 'static,
    {
        self.inner.wait(fut)
    }

    #[doc(hidden)]
    #[inline]
    fn waiting(&self) -> bool {
        self.inner.waiting()
            || self.inner.state() == ActorState::Stopping
            || self.inner.state() == ActorState::Stopped
    }

    fn cancel_future(&mut self, handle: SpawnHandle) -> bool {
        self.inner.cancel_future(handle)
    }

    #[inline]
    fn address(&self) -> Addr<A> {
        self.inner.address()
    }
}

impl<A> WebsocketContext<A>
where
    A: Actor<Context = Self>,
{
    #[inline]
    /// Create a new Websocket context from a request and an actor
    pub fn create<S>(actor: A, stream: S) -> impl Stream<Item = Bytes, Error = Error>
    where
        A: StreamHandler<Message, ProtocolError>,
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        let (_, stream) = WebsocketContext::create_with_addr(actor, stream);
        stream
    }

    #[inline]
    /// Create a new Websocket context from a request and an actor.
    ///
    /// Returns a pair, where the first item is an addr for the created actor,
    /// and the second item is a stream intended to be set as part of the
    /// response via `HttpResponseBuilder::streaming()`.
    pub fn create_with_addr<S>(
        actor: A,
        stream: S,
    ) -> (Addr<A>, impl Stream<Item = Bytes, Error = Error>)
    where
        A: StreamHandler<Message, ProtocolError>,
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        let mb = Mailbox::default();
        let mut ctx = WebsocketContext {
            inner: ContextParts::new(mb.sender_producer()),
            messages: VecDeque::new(),
        };
        ctx.add_stream(WsStream::new(stream, Codec::new()));

        let addr = ctx.address();

        (addr, WebsocketContextFut::new(ctx, actor, mb, Codec::new()))
    }

    #[inline]
    /// Create a new Websocket context from a request, an actor, and a codec
    pub fn with_codec<S>(
        actor: A,
        stream: S,
        codec: Codec,
    ) -> impl Stream<Item = Bytes, Error = Error>
    where
        A: StreamHandler<Message, ProtocolError>,
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        let mb = Mailbox::default();
        let mut ctx = WebsocketContext {
            inner: ContextParts::new(mb.sender_producer()),
            messages: VecDeque::new(),
        };
        ctx.add_stream(WsStream::new(stream, codec));

        WebsocketContextFut::new(ctx, actor, mb, codec)
    }

    /// Create a new Websocket context
    pub fn with_factory<S, F>(
        stream: S,
        f: F,
    ) -> impl Stream<Item = Bytes, Error = Error>
    where
        F: FnOnce(&mut Self) -> A + 'static,
        A: StreamHandler<Message, ProtocolError>,
        S: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        let mb = Mailbox::default();
        let mut ctx = WebsocketContext {
            inner: ContextParts::new(mb.sender_producer()),
            messages: VecDeque::new(),
        };
        ctx.add_stream(WsStream::new(stream, Codec::new()));

        let act = f(&mut ctx);

        WebsocketContextFut::new(ctx, act, mb, Codec::new())
    }
}

impl<A> WebsocketContext<A>
where
    A: Actor<Context = Self>,
{
    /// Write payload
    ///
    /// This is a low-level function that accepts framed messages that should
    /// be created using `Frame::message()`. If you want to send text or binary
    /// data you should prefer the `text()` or `binary()` convenience functions
    /// that handle the framing for you.
    #[inline]
    pub fn write_raw(&mut self, msg: Message) {
        self.messages.push_back(Some(msg));
    }

    /// Send text frame
    #[inline]
    pub fn text<T: Into<String>>(&mut self, text: T) {
        self.write_raw(Message::Text(text.into()));
    }

    /// Send binary frame
    #[inline]
    pub fn binary<B: Into<Bytes>>(&mut self, data: B) {
        self.write_raw(Message::Binary(data.into()));
    }

    /// Send ping frame
    #[inline]
    pub fn ping(&mut self, message: &str) {
        self.write_raw(Message::Ping(message.to_string()));
    }

    /// Send pong frame
    #[inline]
    pub fn pong(&mut self, message: &str) {
        self.write_raw(Message::Pong(message.to_string()));
    }

    /// Send close frame
    #[inline]
    pub fn close(&mut self, reason: Option<CloseReason>) {
        self.write_raw(Message::Close(reason));
    }

    /// Handle of the running future
    ///
    /// SpawnHandle is the handle returned by `AsyncContext::spawn()` method.
    pub fn handle(&self) -> SpawnHandle {
        self.inner.curr_handle()
    }

    /// Set mailbox capacity
    ///
    /// By default mailbox capacity is 16 messages.
    pub fn set_mailbox_capacity(&mut self, cap: usize) {
        self.inner.set_mailbox_capacity(cap)
    }
}

impl<A> AsyncContextParts<A> for WebsocketContext<A>
where
    A: Actor<Context = Self>,
{
    fn parts(&mut self) -> &mut ContextParts<A> {
        &mut self.inner
    }
}

struct WebsocketContextFut<A>
where
    A: Actor<Context = WebsocketContext<A>>,
{
    fut: ContextFut<A, WebsocketContext<A>>,
    encoder: Codec,
    buf: BytesMut,
    closed: bool,
}

impl<A> WebsocketContextFut<A>
where
    A: Actor<Context = WebsocketContext<A>>,
{
    fn new(ctx: WebsocketContext<A>, act: A, mailbox: Mailbox<A>, codec: Codec) -> Self {
        let fut = ContextFut::new(ctx, act, mailbox);
        WebsocketContextFut {
            fut,
            encoder: codec,
            buf: BytesMut::new(),
            closed: false,
        }
    }
}

impl<A> Stream for WebsocketContextFut<A>
where
    A: Actor<Context = WebsocketContext<A>>,
{
    type Item = Bytes;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Bytes>, Error> {
        if self.fut.alive() && self.fut.poll().is_err() {
            return Err(ErrorInternalServerError("error"));
        }

        // encode messages
        while let Some(item) = self.fut.ctx().messages.pop_front() {
            if let Some(msg) = item {
                self.encoder.encode(msg, &mut self.buf)?;
            } else {
                self.closed = true;
                break;
            }
        }

        if !self.buf.is_empty() {
            Ok(Async::Ready(Some(self.buf.take().freeze())))
        } else if self.fut.alive() && !self.closed {
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(None))
        }
    }
}

impl<A, M> ToEnvelope<A, M> for WebsocketContext<A>
where
    A: Actor<Context = WebsocketContext<A>> + Handler<M>,
    M: ActixMessage + Send + 'static,
    M::Result: Send,
{
    fn pack(msg: M, tx: Option<Sender<M::Result>>) -> Envelope<A> {
        Envelope::new(msg, tx)
    }
}

struct WsStream<S> {
    stream: S,
    decoder: Codec,
    buf: BytesMut,
    closed: bool,
}

impl<S> WsStream<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    fn new(stream: S, codec: Codec) -> Self {
        Self {
            stream,
            decoder: codec,
            buf: BytesMut::new(),
            closed: false,
        }
    }
}

impl<S> Stream for WsStream<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Message;
    type Error = ProtocolError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if !self.closed {
            loop {
                match self.stream.poll() {
                    Ok(Async::Ready(Some(chunk))) => {
                        self.buf.extend_from_slice(&chunk[..]);
                    }
                    Ok(Async::Ready(None)) => {
                        self.closed = true;
                        break;
                    }
                    Ok(Async::NotReady) => break,
                    Err(e) => {
                        return Err(ProtocolError::Io(io::Error::new(
                            io::ErrorKind::Other,
                            format!("{}", e),
                        )));
                    }
                }
            }
        }

        match self.decoder.decode(&mut self.buf)? {
            None => {
                if self.closed {
                    Ok(Async::Ready(None))
                } else {
                    Ok(Async::NotReady)
                }
            }
            Some(frm) => {
                let msg = match frm {
                    Frame::Text(data) => {
                        if let Some(data) = data {
                            Message::Text(
                                std::str::from_utf8(&data)
                                    .map_err(|_| ProtocolError::BadEncoding)?
                                    .to_string(),
                            )
                        } else {
                            Message::Text(String::new())
                        }
                    }
                    Frame::Binary(data) => Message::Binary(
                        data.map(|b| b.freeze()).unwrap_or_else(Bytes::new),
                    ),
                    Frame::Ping(s) => Message::Ping(s),
                    Frame::Pong(s) => Message::Pong(s),
                    Frame::Close(reason) => Message::Close(reason),
                };
                Ok(Async::Ready(Some(msg)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::{header, Method};
    use actix_web::test::TestRequest;

    #[test]
    fn test_handshake() {
        let req = TestRequest::default()
            .method(Method::POST)
            .to_http_request();
        assert_eq!(
            HandshakeError::GetMethodRequired,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default().to_http_request();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(header::UPGRADE, header::HeaderValue::from_static("test"))
            .to_http_request();
        assert_eq!(
            HandshakeError::NoWebsocketUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::NoConnectionUpgrade,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::NoVersionHeader,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("5"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::UnsupportedVersion,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .to_http_request();
        assert_eq!(
            HandshakeError::BadWebsocketKey,
            handshake(&req).err().unwrap()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            )
            .to_http_request();

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake(&req).unwrap().finish().status()
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("graphql"),
            )
            .to_http_request();

        let protocols = ["graphql"];

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .status()
        );
        assert_eq!(
            Some(&header::HeaderValue::from_static("graphql")),
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .headers()
                .get(&header::SEC_WEBSOCKET_PROTOCOL)
        );

        let req = TestRequest::default()
            .header(
                header::UPGRADE,
                header::HeaderValue::from_static("websocket"),
            )
            .header(
                header::CONNECTION,
                header::HeaderValue::from_static("upgrade"),
            )
            .header(
                header::SEC_WEBSOCKET_VERSION,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_KEY,
                header::HeaderValue::from_static("13"),
            )
            .header(
                header::SEC_WEBSOCKET_PROTOCOL,
                header::HeaderValue::from_static("p1, p2, p3"),
            )
            .to_http_request();

        let protocols = vec!["p3", "p2"];

        assert_eq!(
            StatusCode::SWITCHING_PROTOCOLS,
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .status()
        );
        assert_eq!(
            Some(&header::HeaderValue::from_static("p2")),
            handshake_with_protocols(&req, &protocols)
                .unwrap()
                .finish()
                .headers()
                .get(&header::SEC_WEBSOCKET_PROTOCOL)
        );
    }
}

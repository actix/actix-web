extern crate actix;

use bytes::Bytes;
use futures::sync::oneshot::{self, Sender};
use futures::{Async, Future, Poll, Stream};
use smallvec::SmallVec;

use self::actix::dev::{
    AsyncContextParts, ContextFut, ContextParts, Envelope, Mailbox, StreamHandler,
    ToEnvelope,
};
use self::actix::fut::ActorFuture;
use self::actix::{
    Actor, ActorContext, ActorState, Addr, AsyncContext, Handler,
    Message as ActixMessage, SpawnHandle,
};

use body::{Binary, Body};
use context::{ActorHttpContext, Drain, Frame as ContextFrame};
use error::{Error, ErrorInternalServerError, PayloadError};
use httprequest::HttpRequest;

use ws::frame::{Frame, FramedMessage};
use ws::proto::{CloseReason, OpCode};
use ws::{Message, ProtocolError, WsStream, WsWriter};

/// Execution context for `WebSockets` actors
pub struct WebsocketContext<A, S = ()>
where
    A: Actor<Context = WebsocketContext<A, S>>,
{
    inner: ContextParts<A>,
    stream: Option<SmallVec<[ContextFrame; 4]>>,
    request: HttpRequest<S>,
    disconnected: bool,
}

impl<A, S> ActorContext for WebsocketContext<A, S>
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

impl<A, S> AsyncContext<A> for WebsocketContext<A, S>
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

impl<A, S: 'static> WebsocketContext<A, S>
where
    A: Actor<Context = Self>,
{
    #[inline]
    /// Create a new Websocket context from a request and an actor
    pub fn create<P>(req: HttpRequest<S>, actor: A, stream: WsStream<P>) -> Body
    where
        A: StreamHandler<Message, ProtocolError>,
        P: Stream<Item = Bytes, Error = PayloadError> + 'static,
    {
        let mb = Mailbox::default();
        let mut ctx = WebsocketContext {
            inner: ContextParts::new(mb.sender_producer()),
            stream: None,
            request: req,
            disconnected: false,
        };
        ctx.add_stream(stream);

        Body::Actor(Box::new(WebsocketContextFut::new(ctx, actor, mb)))
    }

    /// Create a new Websocket context
    pub fn with_factory<F>(req: HttpRequest<S>, f: F) -> Body
    where
        F: FnOnce(&mut Self) -> A + 'static,
    {
        let mb = Mailbox::default();
        let mut ctx = WebsocketContext {
            inner: ContextParts::new(mb.sender_producer()),
            stream: None,
            request: req,
            disconnected: false,
        };

        let act = f(&mut ctx);
        Body::Actor(Box::new(WebsocketContextFut::new(ctx, act, mb)))
    }
}

impl<A, S> WebsocketContext<A, S>
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
    pub fn write_raw(&mut self, data: FramedMessage) {
        if !self.disconnected {
            if self.stream.is_none() {
                self.stream = Some(SmallVec::new());
            }
            let stream = self.stream.as_mut().unwrap();
            stream.push(ContextFrame::Chunk(Some(data.0)));
        } else {
            warn!("Trying to write to disconnected response");
        }
    }

    /// Shared application state
    #[inline]
    pub fn state(&self) -> &S {
        self.request.state()
    }

    /// Incoming request
    #[inline]
    pub fn request(&mut self) -> &mut HttpRequest<S> {
        &mut self.request
    }

    /// Returns drain future
    pub fn drain(&mut self) -> Drain<A> {
        let (tx, rx) = oneshot::channel();
        self.add_frame(ContextFrame::Drain(tx));
        Drain::new(rx)
    }

    /// Send text frame
    #[inline]
    pub fn text<T: Into<Binary>>(&mut self, text: T) {
        self.write_raw(Frame::message(text.into(), OpCode::Text, true, false));
    }

    /// Send binary frame
    #[inline]
    pub fn binary<B: Into<Binary>>(&mut self, data: B) {
        self.write_raw(Frame::message(data, OpCode::Binary, true, false));
    }

    /// Send ping frame
    #[inline]
    pub fn ping(&mut self, message: &str) {
        self.write_raw(Frame::message(
            Vec::from(message),
            OpCode::Ping,
            true,
            false,
        ));
    }

    /// Send pong frame
    #[inline]
    pub fn pong(&mut self, message: &str) {
        self.write_raw(Frame::message(
            Vec::from(message),
            OpCode::Pong,
            true,
            false,
        ));
    }

    /// Send close frame
    #[inline]
    pub fn close(&mut self, reason: Option<CloseReason>) {
        self.write_raw(Frame::close(reason, false));
    }

    /// Check if connection still open
    #[inline]
    pub fn connected(&self) -> bool {
        !self.disconnected
    }

    #[inline]
    fn add_frame(&mut self, frame: ContextFrame) {
        if self.stream.is_none() {
            self.stream = Some(SmallVec::new());
        }
        if let Some(s) = self.stream.as_mut() {
            s.push(frame)
        }
    }

    /// Handle of the running future
    ///
    /// SpawnHandle is the handle returned by `AsyncContext::spawn()` method.
    pub fn handle(&self) -> SpawnHandle {
        self.inner.curr_handle()
    }
}

impl<A, S> WsWriter for WebsocketContext<A, S>
where
    A: Actor<Context = Self>,
    S: 'static,
{
    /// Send text frame
    #[inline]
    fn send_text<T: Into<Binary>>(&mut self, text: T) {
        self.text(text)
    }

    /// Send binary frame
    #[inline]
    fn send_binary<B: Into<Binary>>(&mut self, data: B) {
        self.binary(data)
    }

    /// Send ping frame
    #[inline]
    fn send_ping(&mut self, message: &str) {
        self.ping(message)
    }

    /// Send pong frame
    #[inline]
    fn send_pong(&mut self, message: &str) {
        self.pong(message)
    }

    /// Send close frame
    #[inline]
    fn send_close(&mut self, reason: Option<CloseReason>) {
        self.close(reason)
    }
}

impl<A, S> AsyncContextParts<A> for WebsocketContext<A, S>
where
    A: Actor<Context = Self>,
{
    fn parts(&mut self) -> &mut ContextParts<A> {
        &mut self.inner
    }
}

struct WebsocketContextFut<A, S>
where
    A: Actor<Context = WebsocketContext<A, S>>,
{
    fut: ContextFut<A, WebsocketContext<A, S>>,
}

impl<A, S> WebsocketContextFut<A, S>
where
    A: Actor<Context = WebsocketContext<A, S>>,
{
    fn new(ctx: WebsocketContext<A, S>, act: A, mailbox: Mailbox<A>) -> Self {
        let fut = ContextFut::new(ctx, act, mailbox);
        WebsocketContextFut { fut }
    }
}

impl<A, S> ActorHttpContext for WebsocketContextFut<A, S>
where
    A: Actor<Context = WebsocketContext<A, S>>,
    S: 'static,
{
    #[inline]
    fn disconnected(&mut self) {
        self.fut.ctx().disconnected = true;
        self.fut.ctx().stop();
    }

    fn poll(&mut self) -> Poll<Option<SmallVec<[ContextFrame; 4]>>, Error> {
        if self.fut.alive() && self.fut.poll().is_err() {
            return Err(ErrorInternalServerError("error"));
        }

        // frames
        if let Some(data) = self.fut.ctx().stream.take() {
            Ok(Async::Ready(Some(data)))
        } else if self.fut.alive() {
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(None))
        }
    }
}

impl<A, M, S> ToEnvelope<A, M> for WebsocketContext<A, S>
where
    A: Actor<Context = WebsocketContext<A, S>> + Handler<M>,
    M: ActixMessage + Send + 'static,
    M::Result: Send,
{
    fn pack(msg: M, tx: Option<Sender<M::Result>>) -> Envelope<A> {
        Envelope::new(msg, tx)
    }
}

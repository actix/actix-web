use std::mem;
use futures::{Async, Poll};
use futures::sync::oneshot::Sender;
use futures::unsync::oneshot;
use smallvec::SmallVec;

use actix::{Actor, ActorState, ActorContext, AsyncContext,
            Address, SyncAddress, Handler, Subscriber, ResponseType, SpawnHandle};
use actix::fut::ActorFuture;
use actix::dev::{queue, AsyncContextApi,
                 ContextImpl, ContextProtocol, Envelope, ToEnvelope, RemoteEnvelope};

use body::{Body, Binary};
use error::{Error, Result, ErrorInternalServerError};
use httprequest::HttpRequest;
use context::{Frame as ContextFrame, ActorHttpContext, Drain};

use ws::frame::Frame;
use ws::proto::{OpCode, CloseCode};


/// Http actor execution context
pub struct WebsocketContext<A, S=()> where A: Actor<Context=WebsocketContext<A, S>>,
{
    inner: ContextImpl<A>,
    stream: Option<SmallVec<[ContextFrame; 4]>>,
    request: HttpRequest<S>,
    disconnected: bool,
}

impl<A, S> ActorContext for WebsocketContext<A, S> where A: Actor<Context=Self>
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

impl<A, S> AsyncContext<A> for WebsocketContext<A, S> where A: Actor<Context=Self>
{
    fn spawn<F>(&mut self, fut: F) -> SpawnHandle
        where F: ActorFuture<Item=(), Error=(), Actor=A> + 'static
    {
        self.inner.spawn(fut)
    }

    fn wait<F>(&mut self, fut: F)
        where F: ActorFuture<Item=(), Error=(), Actor=A> + 'static
    {
        self.inner.wait(fut)
    }

    #[doc(hidden)]
    #[inline]
    fn waiting(&self) -> bool {
        self.inner.waiting() || self.inner.state() == ActorState::Stopping ||
            self.inner.state() == ActorState::Stopped
    }

    fn cancel_future(&mut self, handle: SpawnHandle) -> bool {
        self.inner.cancel_future(handle)
    }
}

#[doc(hidden)]
impl<A, S> AsyncContextApi<A> for WebsocketContext<A, S> where A: Actor<Context=Self> {
    #[inline]
    fn unsync_sender(&mut self) -> queue::unsync::UnboundedSender<ContextProtocol<A>> {
        self.inner.unsync_sender()
    }

    #[inline]
    fn unsync_address(&mut self) -> Address<A> {
        self.inner.unsync_address()
    }

    #[inline]
    fn sync_address(&mut self) -> SyncAddress<A> {
        self.inner.sync_address()
    }
}

impl<A, S: 'static> WebsocketContext<A, S> where A: Actor<Context=Self> {

    #[inline]
    pub fn new(req: HttpRequest<S>, actor: A) -> WebsocketContext<A, S> {
        WebsocketContext::from_request(req).actor(actor)
    }

    pub fn from_request(req: HttpRequest<S>) -> WebsocketContext<A, S> {
        WebsocketContext {
            inner: ContextImpl::new(None),
            stream: None,
            request: req,
            disconnected: false,
        }
    }

    #[inline]
    pub fn actor(mut self, actor: A) -> WebsocketContext<A, S> {
        self.inner.set_actor(actor);
        self
    }
}

impl<A, S> WebsocketContext<A, S> where A: Actor<Context=Self> {

    /// Write payload
    #[inline]
    fn write<B: Into<Binary>>(&mut self, data: B) {
        if !self.disconnected {
            self.add_frame(ContextFrame::Chunk(Some(data.into())));
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

    /// Send text frame
    pub fn text(&mut self, text: &str) {
        let mut frame = Frame::message(Vec::from(text), OpCode::Text, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send binary frame
    pub fn binary<B: Into<Binary>>(&mut self, data: B) {
        let mut frame = Frame::message(data, OpCode::Binary, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send ping frame
    pub fn ping(&mut self, message: &str) {
        let mut frame = Frame::message(Vec::from(message), OpCode::Ping, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send pong frame
    pub fn pong(&mut self, message: &str) {
        let mut frame = Frame::message(Vec::from(message), OpCode::Pong, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        self.write(buf);
    }

    /// Send close frame
    pub fn close(&mut self, code: CloseCode, reason: &str) {
        let mut frame = Frame::close(code, reason);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();
        self.write(buf);
    }

    /// Returns drain future
    pub fn drain(&mut self) -> Drain<A> {
        let (tx, rx) = oneshot::channel();
        self.inner.modify();
        self.add_frame(ContextFrame::Drain(tx));
        Drain::new(rx)
    }

    /// Check if connection still open
    #[inline]
    pub fn connected(&self) -> bool {
        !self.disconnected
    }

    fn add_frame(&mut self, frame: ContextFrame) {
        if self.stream.is_none() {
            self.stream = Some(SmallVec::new());
        }
        self.stream.as_mut().map(|s| s.push(frame));
    }
}

impl<A, S> WebsocketContext<A, S> where A: Actor<Context=Self> {

    #[inline]
    #[doc(hidden)]
    pub fn subscriber<M>(&mut self) -> Box<Subscriber<M>>
        where A: Handler<M>, M: ResponseType + 'static
    {
        self.inner.subscriber()
    }

    #[inline]
    #[doc(hidden)]
    pub fn sync_subscriber<M>(&mut self) -> Box<Subscriber<M> + Send>
        where A: Handler<M>,
              M: ResponseType + Send + 'static, M::Item: Send, M::Error: Send,
    {
        self.inner.sync_subscriber()
    }
}

impl<A, S> ActorHttpContext for WebsocketContext<A, S> where A: Actor<Context=Self>, S: 'static {

    #[inline]
    fn disconnected(&mut self) {
        self.disconnected = true;
        self.stop();
    }

    fn poll(&mut self) -> Poll<Option<SmallVec<[ContextFrame; 4]>>, Error> {
        let ctx: &mut WebsocketContext<A, S> = unsafe {
            mem::transmute(self as &mut WebsocketContext<A, S>)
        };

        if self.inner.alive() {
            match self.inner.poll(ctx) {
                Ok(Async::NotReady) | Ok(Async::Ready(())) => (),
                Err(_) => return Err(ErrorInternalServerError("error").into()),
            }
        }

        // frames
        if let Some(data) = self.stream.take() {
            Ok(Async::Ready(Some(data)))
        } else if self.inner.alive() {
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(None))
        }
    }
}

impl<A, S> ToEnvelope<A> for WebsocketContext<A, S>
    where A: Actor<Context=WebsocketContext<A, S>>,
{
    #[inline]
    fn pack<M>(msg: M, tx: Option<Sender<Result<M::Item, M::Error>>>,
               channel_on_drop: bool) -> Envelope<A>
        where A: Handler<M>,
              M: ResponseType + Send + 'static, M::Item: Send, M::Error: Send {
        RemoteEnvelope::new(msg, tx, channel_on_drop).into()
    }
}

impl<A, S> From<WebsocketContext<A, S>> for Body
    where A: Actor<Context=WebsocketContext<A, S>>, S: 'static
{
    fn from(ctx: WebsocketContext<A, S>) -> Body {
        Body::Actor(Box::new(ctx))
    }
}

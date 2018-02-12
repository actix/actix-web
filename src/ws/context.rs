use std::mem;
use futures::{Async, Poll};
use futures::sync::oneshot::Sender;
use futures::unsync::oneshot;
use smallvec::SmallVec;

use actix::{Actor, ActorState, ActorContext, AsyncContext,
            Addr, Handler, ResponseType, SpawnHandle, MessageResult, Syn, Unsync};
use actix::fut::ActorFuture;
use actix::dev::{ContextImpl, ToEnvelope, RemoteEnvelope};

use body::{Body, Binary};
use error::{Error, ErrorInternalServerError};
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

    #[doc(hidden)]
    #[inline]
    fn unsync_address(&mut self) -> Addr<Unsync<A>> {
        self.inner.unsync_address()
    }

    #[doc(hidden)]
    #[inline]
    fn sync_address(&mut self) -> Addr<Syn<A>> {
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
    fn write(&mut self, data: Binary) {
        if !self.disconnected {
            if self.stream.is_none() {
                self.stream = Some(SmallVec::new());
            }
            let stream = self.stream.as_mut().unwrap();
            stream.push(ContextFrame::Chunk(Some(data)));
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
    #[inline]
    pub fn text<T: Into<String>>(&mut self, text: T) {
        self.write(Frame::message(text.into(), OpCode::Text, true, false));
    }

    /// Send binary frame
    #[inline]
    pub fn binary<B: Into<Binary>>(&mut self, data: B) {
        self.write(Frame::message(data, OpCode::Binary, true, false));
    }

    /// Send ping frame
    #[inline]
    pub fn ping(&mut self, message: &str) {
        self.write(Frame::message(Vec::from(message), OpCode::Ping, true, false));
    }

    /// Send pong frame
    #[inline]
    pub fn pong(&mut self, message: &str) {
        self.write(Frame::message(Vec::from(message), OpCode::Pong, true, false));
    }

    /// Send close frame
    #[inline]
    pub fn close(&mut self, code: CloseCode, reason: &str) {
        self.write(Frame::close(code, reason, false));
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

    #[inline]
    fn add_frame(&mut self, frame: ContextFrame) {
        if self.stream.is_none() {
            self.stream = Some(SmallVec::new());
        }
        self.stream.as_mut().map(|s| s.push(frame));
    }

    /// Handle of the running future
    ///
    /// SpawnHandle is the handle returned by `AsyncContext::spawn()` method.
    pub fn handle(&self) -> SpawnHandle {
        self.inner.curr_handle()
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

        if self.inner.alive() && self.inner.poll(ctx).is_err() {
            return Err(ErrorInternalServerError("error").into())
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

impl<A, M, S> ToEnvelope<Syn<A>, M> for WebsocketContext<A, S>
    where A: Actor<Context=WebsocketContext<A, S>> + Handler<M>,
          M: ResponseType + Send + 'static, M::Item: Send, M::Error: Send,
{
    fn pack(msg: M, tx: Option<Sender<MessageResult<M>>>) -> Syn<A> {
        Syn::new(Box::new(RemoteEnvelope::envelope(msg, tx)))
    }
}

impl<A, S> From<WebsocketContext<A, S>> for Body
    where A: Actor<Context=WebsocketContext<A, S>>, S: 'static
{
    fn from(ctx: WebsocketContext<A, S>) -> Body {
        Body::Actor(Box::new(ctx))
    }
}

use std;
use std::marker::PhantomData;
use futures::{Async, Future, Poll};
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


pub trait ActorHttpContext: 'static {
    fn disconnected(&mut self);
    fn poll(&mut self) -> Poll<Option<SmallVec<[Frame; 2]>>, Error>;
}

#[derive(Debug)]
pub enum Frame {
    Chunk(Option<Binary>),
    Drain(oneshot::Sender<()>),
}

/// Http actor execution context
pub struct HttpContext<A, S=()> where A: Actor<Context=HttpContext<A, S>>,
{
    inner: ContextImpl<A>,
    stream: Option<SmallVec<[Frame; 2]>>,
    request: HttpRequest<S>,
    disconnected: bool,
}

impl<A, S> ActorContext for HttpContext<A, S> where A: Actor<Context=Self>
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

impl<A, S> AsyncContext<A> for HttpContext<A, S> where A: Actor<Context=Self>
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
    fn cancel_future(&mut self, handle: SpawnHandle) -> bool {
        self.inner.cancel_future(handle)
    }
}

#[doc(hidden)]
impl<A, S> AsyncContextApi<A> for HttpContext<A, S> where A: Actor<Context=Self> {
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

impl<A, S: 'static> HttpContext<A, S> where A: Actor<Context=Self> {

    #[inline]
    pub fn new(req: HttpRequest<S>, actor: A) -> HttpContext<A, S> {
        HttpContext::from_request(req).actor(actor)
    }
    pub fn from_request(req: HttpRequest<S>) -> HttpContext<A, S> {
        HttpContext {
            inner: ContextImpl::new(None),
            stream: None,
            request: req,
            disconnected: false,
        }
    }
    #[inline]
    pub fn actor(mut self, actor: A) -> HttpContext<A, S> {
        self.inner.set_actor(actor);
        self
    }
}

impl<A, S> HttpContext<A, S> where A: Actor<Context=Self> {

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

    /// Write payload
    #[inline]
    pub fn write<B: Into<Binary>>(&mut self, data: B) {
        if !self.disconnected {
            self.add_frame(Frame::Chunk(Some(data.into())));
        } else {
            warn!("Trying to write to disconnected response");
        }
    }

    /// Indicate end of streaming payload. Also this method calls `Self::close`.
    #[inline]
    pub fn write_eof(&mut self) {
        self.add_frame(Frame::Chunk(None));
    }

    /// Returns drain future
    pub fn drain(&mut self) -> Drain<A> {
        let (tx, rx) = oneshot::channel();
        self.inner.modify();
        self.add_frame(Frame::Drain(tx));
        Drain::new(rx)
    }

    /// Check if connection still open
    #[inline]
    pub fn connected(&self) -> bool {
        !self.disconnected
    }

    #[inline]
    fn add_frame(&mut self, frame: Frame) {
        if self.stream.is_none() {
            self.stream = Some(SmallVec::new());
        }
        self.stream.as_mut().map(|s| s.push(frame));
    }
}

impl<A, S> HttpContext<A, S> where A: Actor<Context=Self> {

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

impl<A, S> ActorHttpContext for HttpContext<A, S> where A: Actor<Context=Self>, S: 'static {

    #[inline]
    fn disconnected(&mut self) {
        self.disconnected = true;
        self.stop();
    }

    fn poll(&mut self) -> Poll<Option<SmallVec<[Frame; 2]>>, Error> {
        let ctx: &mut HttpContext<A, S> = unsafe {
            std::mem::transmute(self as &mut HttpContext<A, S>)
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

impl<A, S> ToEnvelope<A> for HttpContext<A, S>
    where A: Actor<Context=HttpContext<A, S>>,
{
    #[inline]
    fn pack<M>(msg: M, tx: Option<Sender<Result<M::Item, M::Error>>>,
               channel_on_drop: bool) -> Envelope<A>
        where A: Handler<M>,
              M: ResponseType + Send + 'static, M::Item: Send, M::Error: Send
    {
        RemoteEnvelope::new(msg, tx, channel_on_drop).into()
    }
}

impl<A, S> From<HttpContext<A, S>> for Body
    where A: Actor<Context=HttpContext<A, S>>,
          S: 'static
{
    fn from(ctx: HttpContext<A, S>) -> Body {
        Body::Actor(Box::new(ctx))
    }
}

pub struct Drain<A> {
    fut: oneshot::Receiver<()>,
    _a: PhantomData<A>,
}

impl<A> Drain<A> {
    pub fn new(fut: oneshot::Receiver<()>) -> Self {
        Drain {
            fut: fut,
            _a: PhantomData
        }
    }
}

impl<A: Actor> ActorFuture for Drain<A> {
    type Item = ();
    type Error = ();
    type Actor = A;

    #[inline]
    fn poll(&mut self,
            _: &mut A,
            _: &mut <Self::Actor as Actor>::Context) -> Poll<Self::Item, Self::Error>
    {
        self.fut.poll().map_err(|_| ())
    }
}

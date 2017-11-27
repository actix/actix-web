use std;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use futures::{Async, Future, Stream, Poll};
use futures::sync::oneshot::Sender;

use actix::{Actor, ActorState, ActorContext, AsyncContext,
            Handler, Subscriber, ResponseType};
use actix::fut::ActorFuture;
use actix::dev::{AsyncContextApi, ActorAddressCell, ActorItemsCell, ActorWaitCell, SpawnHandle,
                 Envelope, ToEnvelope, RemoteEnvelope};

use task::{IoContext, DrainFut};
use body::Binary;
use error::Error;
use route::{Route, Frame};
use httpresponse::HttpResponse;


/// Http actor execution context
pub struct HttpContext<A> where A: Actor<Context=HttpContext<A>> + Route,
{
    act: Option<A>,
    state: ActorState,
    modified: bool,
    items: ActorItemsCell<A>,
    address: ActorAddressCell<A>,
    stream: VecDeque<Frame>,
    wait: ActorWaitCell<A>,
    app_state: Rc<<A as Route>::State>,
    disconnected: bool,
}

impl<A> IoContext for HttpContext<A> where A: Actor<Context=Self> + Route {

    fn disconnected(&mut self) {
        self.items.stop();
        self.disconnected = true;
        if self.state == ActorState::Running {
            self.state = ActorState::Stopping;
        }
    }
}

impl<A> ActorContext for HttpContext<A> where A: Actor<Context=Self> + Route
{
    /// Stop actor execution
    fn stop(&mut self) {
        self.stream.push_back(Frame::Payload(None));
        self.items.stop();
        self.address.close();
        if self.state == ActorState::Running {
            self.state = ActorState::Stopping;
        }
    }

    /// Terminate actor execution
    fn terminate(&mut self) {
        self.address.close();
        self.items.close();
        self.state = ActorState::Stopped;
    }

    /// Actor execution state
    fn state(&self) -> ActorState {
        self.state
    }
}

impl<A> AsyncContext<A> for HttpContext<A> where A: Actor<Context=Self> + Route
{
    fn spawn<F>(&mut self, fut: F) -> SpawnHandle
        where F: ActorFuture<Item=(), Error=(), Actor=A> + 'static
    {
        self.modified = true;
        self.items.spawn(fut)
    }

    fn wait<F>(&mut self, fut: F)
        where F: ActorFuture<Item=(), Error=(), Actor=A> + 'static
    {
        self.modified = true;
        self.wait.add(fut);
    }

    fn cancel_future(&mut self, handle: SpawnHandle) -> bool {
        self.modified = true;
        self.items.cancel_future(handle)
    }

    fn cancel_future_on_stop(&mut self, handle: SpawnHandle) {
        self.items.cancel_future_on_stop(handle)
    }
}

#[doc(hidden)]
impl<A> AsyncContextApi<A> for HttpContext<A> where A: Actor<Context=Self> + Route {
    fn address_cell(&mut self) -> &mut ActorAddressCell<A> {
        &mut self.address
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

    pub fn new(state: Rc<<A as Route>::State>) -> HttpContext<A>
    {
        HttpContext {
            act: None,
            state: ActorState::Started,
            modified: false,
            items: ActorItemsCell::default(),
            address: ActorAddressCell::default(),
            wait: ActorWaitCell::default(),
            stream: VecDeque::new(),
            app_state: state,
            disconnected: false,
        }
    }

    pub(crate) fn set_actor(&mut self, act: A) {
        self.act = Some(act)
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

    /// Shared application state
    pub fn state(&self) -> &<A as Route>::State {
        &self.app_state
    }

    /// Start response processing
    pub fn start<R: Into<HttpResponse>>(&mut self, response: R) {
        self.stream.push_back(Frame::Message(response.into()))
    }

    /// Write payload
    pub fn write<B: Into<Binary>>(&mut self, data: B) {
        self.stream.push_back(Frame::Payload(Some(data.into())))
    }

    /// Indicate end of streamimng payload. Also this method calls `Self::close`.
    pub fn write_eof(&mut self) {
        self.stop();
    }

    /// Returns drain future
    pub fn drain(&mut self) -> Drain<A> {
        let fut = Rc::new(RefCell::new(DrainFut::default()));
        self.stream.push_back(Frame::Drain(Rc::clone(&fut)));
        self.modified = true;
        Drain{ a: PhantomData, inner: fut }
    }

    /// Check if connection still open
    pub fn connected(&self) -> bool {
        !self.disconnected
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

    #[doc(hidden)]
    pub fn subscriber<M>(&mut self) -> Box<Subscriber<M>>
        where A: Handler<M>,
              M: ResponseType + 'static,
    {
        Box::new(self.address.unsync_address())
    }

    #[doc(hidden)]
    pub fn sync_subscriber<M>(&mut self) -> Box<Subscriber<M> + Send>
        where A: Handler<M>,
              M: ResponseType + Send + 'static,
              M::Item: Send,
              M::Error: Send,
    {
        Box::new(self.address.sync_address())
    }
}

#[doc(hidden)]
impl<A> Stream for HttpContext<A> where A: Actor<Context=Self> + Route
{
    type Item = Frame;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Frame>, Error> {
        if self.act.is_none() {
            return Ok(Async::NotReady)
        }
        let act: &mut A = unsafe {
            std::mem::transmute(self.act.as_mut().unwrap() as &mut A)
        };
        let ctx: &mut HttpContext<A> = unsafe {
            std::mem::transmute(self as &mut HttpContext<A>)
        };

        // update state
        match self.state {
            ActorState::Started => {
                Actor::started(act, ctx);
                self.state = ActorState::Running;
            },
            ActorState::Stopping => {
                Actor::stopping(act, ctx);
            }
            _ => ()
        }

        let mut prep_stop = false;
        loop {
            self.modified = false;

            // check wait futures
            if self.wait.poll(act, ctx) {
                // get frame
                if let Some(frame) = self.stream.pop_front() {
                    return Ok(Async::Ready(Some(frame)))
                }
                return Ok(Async::NotReady)
            }

            // incoming messages
            self.address.poll(act, ctx);

            // spawned futures and streams
            self.items.poll(act, ctx);

            // are we done
            if self.modified {
                continue
            }

            // get frame
            if let Some(frame) = self.stream.pop_front() {
                return Ok(Async::Ready(Some(frame)))
            }

            // check state
            match self.state {
                ActorState::Stopped => {
                    self.state = ActorState::Stopped;
                    Actor::stopped(act, ctx);
                    return Ok(Async::Ready(None))
                },
                ActorState::Stopping => {
                    if prep_stop {
                        if self.address.connected() || !self.items.is_empty() {
                            self.state = ActorState::Running;
                            continue
                        } else {
                            self.state = ActorState::Stopped;
                            Actor::stopped(act, ctx);
                            return Ok(Async::Ready(None))
                        }
                    } else {
                        Actor::stopping(act, ctx);
                        prep_stop = true;
                        continue
                    }
                },
                ActorState::Running => {
                    if !self.address.connected() && self.items.is_empty() {
                        self.state = ActorState::Stopping;
                        Actor::stopping(act, ctx);
                        prep_stop = true;
                        continue
                    }
                },
                _ => (),
            }

            return Ok(Async::NotReady)
        }
    }
}

impl<A> ToEnvelope<A> for HttpContext<A>
    where A: Actor<Context=HttpContext<A>> + Route,
{
    fn pack<M>(msg: M, tx: Option<Sender<Result<M::Item, M::Error>>>) -> Envelope<A>
        where A: Handler<M>,
              M: ResponseType + Send + 'static,
              M::Item: Send,
              M::Error: Send
    {
        RemoteEnvelope::new(msg, tx).into()
    }
}


pub struct Drain<A> {
    a: PhantomData<A>,
    inner: Rc<RefCell<DrainFut>>
}

impl<A> ActorFuture for Drain<A>
    where A: Actor
{
    type Item = ();
    type Error = ();
    type Actor = A;

    fn poll(&mut self, _: &mut A, _: &mut <Self::Actor as Actor>::Context) -> Poll<(), ()> {
        self.inner.borrow_mut().poll()
    }
}

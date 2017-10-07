use std;
use std::rc::Rc;
use std::collections::VecDeque;
use futures::{Async, Stream, Poll};

use bytes::Bytes;
use actix::{Actor, ActorState, ActorContext, AsyncActorContext};
use actix::fut::ActorFuture;
use actix::dev::{AsyncContextApi, ActorAddressCell};

use route::{Route, Frame};
use httpmessage::HttpMessage;


/// Actor execution context
pub struct HttpContext<A> where A: Actor<Context=HttpContext<A>> + Route,
{
    act: A,
    state: ActorState,
    items: Vec<Box<ActorFuture<Item=(), Error=(), Actor=A>>>,
    address: ActorAddressCell<A>,
    stream: VecDeque<Frame>,
    app_state: Rc<<A as Route>::State>,
}


impl<A> ActorContext<A> for HttpContext<A> where A: Actor<Context=Self> + Route
{
    /// Stop actor execution
    fn stop(&mut self) {
        self.address.close();
        if self.state == ActorState::Running {
            self.state = ActorState::Stopping;
        }
    }

    /// Terminate actor execution
    fn terminate(&mut self) {
        self.address.close();
        self.items.clear();
        self.state = ActorState::Stopped;
    }

    /// Actor execution state
    fn state(&self) -> ActorState {
        self.state
    }
}

impl<A> AsyncActorContext<A> for HttpContext<A> where A: Actor<Context=Self> + Route
{
    fn spawn<F>(&mut self, fut: F)
        where F: ActorFuture<Item=(), Error=(), Actor=A> + 'static
    {
        if self.state == ActorState::Stopped {
            error!("Context::spawn called for stopped actor.");
        } else {
            self.items.push(Box::new(fut))
        }
    }
}

impl<A> AsyncContextApi<A> for HttpContext<A> where A: Actor<Context=Self> + Route {
    fn address_cell(&mut self) -> &mut ActorAddressCell<A> {
        &mut self.address
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

    pub(crate) fn new(act: A, state: Rc<<A as Route>::State>) -> HttpContext<A>
    {
        HttpContext {
            act: act,
            state: ActorState::Started,
            items: Vec::new(),
            address: ActorAddressCell::new(),
            stream: VecDeque::new(),
            app_state: state,
        }
    }

    pub(crate) fn replace_actor(&mut self, srv: A) -> A {
        std::mem::replace(&mut self.act, srv)
    }
}

impl<A> HttpContext<A> where A: Actor<Context=Self> + Route {

    /// Shared application state
    pub fn state(&self) -> &<A as Route>::State {
        &self.app_state
    }
    
    /// Start response processing
    pub fn start(&mut self, response: HttpMessage) {
        self.stream.push_back(Frame::Message(response))
    }

    /// Write payload
    pub fn write(&mut self, data: Bytes) {
        self.stream.push_back(Frame::Payload(Some(data)))
    }

    /// Completed
    pub fn write_eof(&mut self) {
        self.stream.push_back(Frame::Payload(None))
    }
}

impl<A> Stream for HttpContext<A> where A: Actor<Context=Self> + Route
{
    type Item = Frame;
    type Error = std::io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let ctx: &mut HttpContext<A> = unsafe {
            std::mem::transmute(self as &mut HttpContext<A>)
        };

        // update state
        match self.state {
            ActorState::Started => {
                Actor::started(&mut self.act, ctx);
                self.state = ActorState::Running;
            },
            ActorState::Stopping => {
                Actor::stopping(&mut self.act, ctx);
            }
            _ => ()
        }

        let mut prep_stop = false;
        loop {
            let mut not_ready = true;

            if let Ok(Async::Ready(_)) = self.address.poll(&mut self.act, ctx) {
                not_ready = false
            }

            // check secondary streams
            let mut idx = 0;
            let mut len = self.items.len();
            loop {
                if idx >= len {
                    break
                }

                let (drop, item) = match self.items[idx].poll(&mut self.act, ctx) {
                    Ok(val) => match val {
                        Async::Ready(_) => {
                            not_ready = false;
                            (true, None)
                        }
                        Async::NotReady => (false, None),
                    },
                    Err(_) => (true, None)
                };

                // we have new pollable item
                if let Some(item) = item {
                    self.items.push(item);
                }

                // number of items could be different, context can add more items
                len = self.items.len();

                // item finishes, we need to remove it,
                // replace current item with last item
                if drop {
                    len -= 1;
                    if idx >= len {
                        self.items.pop();
                        break
                    } else {
                        self.items[idx] = self.items.pop().unwrap();
                    }
                } else {
                    idx += 1;
                }
            }

            // are we done
            if !not_ready {
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
                    Actor::stopped(&mut self.act, ctx);
                    return Ok(Async::Ready(None))
                },
                ActorState::Stopping => {
                    if prep_stop {
                        if self.address.connected() || !self.items.is_empty() {
                            self.state = ActorState::Running;
                            continue
                        } else {
                            self.state = ActorState::Stopped;
                            Actor::stopped(&mut self.act, ctx);
                            return Ok(Async::Ready(None))
                        }
                    } else {
                        Actor::stopping(&mut self.act, ctx);
                        prep_stop = true;
                        continue
                    }
                },
                ActorState::Running => {
                    if !self.address.connected() && self.items.is_empty() {
                        self.state = ActorState::Stopping;
                        Actor::stopping(&mut self.act, ctx);
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

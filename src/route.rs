use std::rc::Rc;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::result::Result as StdResult;

use actix::Actor;
use futures::Stream;

use body::Binary;
use error::Error;
use context::HttpContext;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use task::{Task, DrainFut, IoContext};

#[doc(hidden)]
#[derive(Debug)]
pub enum Frame {
    Message(HttpResponse),
    Payload(Option<Binary>),
    Drain(Rc<RefCell<DrainFut>>),
}

impl Frame {
    pub fn eof() -> Frame {
        Frame::Payload(None)
    }
}

/// Trait defines object that could be regestered as route handler
#[allow(unused_variables)]
pub trait Handler<S>: 'static {
    type Result: Into<Reply>;

    /// Handle request
    fn handle(&self, req: HttpRequest<S>) -> Self::Result;

    /// Set route prefix
    fn set_prefix(&mut self, prefix: String) {}
}

/// Handler<S> for Fn()
impl<F, R, S> Handler<S> for F
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Into<Reply> + 'static
{
    type Result = R;

    fn handle(&self, req: HttpRequest<S>) -> R {
        (self)(req)
    }
}

/// Represents response process.
pub struct Reply(ReplyItem);

enum ReplyItem {
    Message(HttpResponse),
    Actor(Box<IoContext<Item=Frame, Error=Error>>),
    Stream(Box<Stream<Item=Frame, Error=Error>>),
}

impl Reply {

    /// Create actor response
    pub fn actor<A, S>(ctx: HttpContext<A, S>) -> Reply
        where A: Actor<Context=HttpContext<A, S>>, S: 'static
    {
        Reply(ReplyItem::Actor(Box::new(ctx)))
    }

    /// Create async response
    pub fn stream<S>(stream: S) -> Reply
        where S: Stream<Item=Frame, Error=Error> + 'static
    {
        Reply(ReplyItem::Stream(Box::new(stream)))
    }

    /// Send response
    pub fn reply<R: Into<HttpResponse>>(response: R) -> Reply {
        Reply(ReplyItem::Message(response.into()))
    }

    pub fn into(self, task: &mut Task)
    {
        match self.0 {
            ReplyItem::Message(msg) => {
                task.reply(msg)
            },
            ReplyItem::Actor(ctx) => {
                task.context(ctx)
            }
            ReplyItem::Stream(stream) => {
                task.stream(stream)
            }
        }
    }
}

impl<T: Into<HttpResponse>> From<T> for Reply
{
    fn from(item: T) -> Self {
        Reply(ReplyItem::Message(item.into()))
    }
}

impl<E: Into<Error>> From<StdResult<Reply, E>> for Reply {
    fn from(res: StdResult<Reply, E>) -> Self {
        match res {
            Ok(val) => val,
            Err(err) => err.into().into(),
        }
    }
}

impl<A: Actor<Context=HttpContext<A, S>>, S: 'static> From<HttpContext<A, S>> for Reply
{
    fn from(item: HttpContext<A, S>) -> Self {
        Reply(ReplyItem::Actor(Box::new(item)))
    }
}

/// Trait defines object that could be regestered as resource route
pub(crate) trait RouteHandler<S>: 'static {
    /// Handle request
    fn handle(&self, req: HttpRequest<S>, task: &mut Task);

    /// Set route prefix
    fn set_prefix(&mut self, _prefix: String) {}
}

/// Route handler wrapper for Handler
pub(crate)
struct WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: Into<Reply>,
          S: 'static,
{
    h: H,
    s: PhantomData<S>,
}

impl<S, H, R> WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: Into<Reply>,
          S: 'static,
{
    pub fn new(h: H) -> Self {
        WrapHandler{h: h, s: PhantomData}
    }
}

impl<S, H, R> RouteHandler<S> for WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: Into<Reply> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>, task: &mut Task) {
        self.h.handle(req).into().into(task)
    }

    fn set_prefix(&mut self, prefix: String) {
        self.h.set_prefix(prefix)
    }
}

/// Async route handler
pub(crate)
struct StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Stream<Item=Frame, Error=Error> + 'static,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Stream<Item=Frame, Error=Error> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        StreamHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Stream<Item=Frame, Error=Error> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>, task: &mut Task) {
        task.stream((self.f)(req))
    }
}

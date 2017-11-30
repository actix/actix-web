use std::marker::PhantomData;
use std::result::Result as StdResult;

use actix::Actor;
use futures::Future;

use error::Error;
use context::HttpContext;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use task::{Task, IoContext};

/// Trait defines object that could be regestered as route handler
#[allow(unused_variables)]
pub trait Handler<S>: 'static {

    /// The type of value that handler will return.
    type Result: Into<Reply>;

    /// Handle request
    fn handle(&self, req: HttpRequest<S>) -> Self::Result;
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
    Actor(Box<IoContext>),
    Future(Box<Future<Item=HttpResponse, Error=Error>>),
}

impl Reply {

    /// Create actor response
    pub fn actor<A, S>(ctx: HttpContext<A, S>) -> Reply
        where A: Actor<Context=HttpContext<A, S>>, S: 'static
    {
        Reply(ReplyItem::Actor(Box::new(ctx)))
    }

    /// Create async response
    pub fn async<F>(fut: F) -> Reply
        where F: Future<Item=HttpResponse, Error=Error> + 'static
    {
        Reply(ReplyItem::Future(Box::new(fut)))
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
            ReplyItem::Future(fut) => {
                task.async(fut)
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
}

/// Async route handler
pub(crate)
struct StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Future<Item=HttpResponse, Error=Error> + 'static,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Future<Item=HttpResponse, Error=Error> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        StreamHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for StreamHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Future<Item=HttpResponse, Error=Error> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>, task: &mut Task) {
        task.async((self.f)(req))
    }
}

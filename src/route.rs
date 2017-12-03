use std::marker::PhantomData;
use std::result::Result as StdResult;

use actix::Actor;
use futures::Future;

use error::Error;
use context::{HttpContext, IoContext};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Trait defines object that could be regestered as route handler
#[allow(unused_variables)]
pub trait Handler<S>: 'static {

    /// The type of value that handler will return.
    type Result: FromRequest;

    /// Handle request
    fn handle(&self, req: HttpRequest<S>) -> Self::Result;
}

pub trait FromRequest {
    fn from_request(self, req: HttpRequest) -> Reply;
}

/// Handler<S> for Fn()
impl<F, R, S> Handler<S> for F
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: FromRequest + 'static
{
    type Result = R;

    fn handle(&self, req: HttpRequest<S>) -> R {
        (self)(req)
    }
}

/// Represents response process.
pub struct Reply(ReplyItem);

pub(crate) enum ReplyItem {
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
    pub fn response<R: Into<HttpResponse>>(response: R) -> Reply {
        Reply(ReplyItem::Message(response.into()))
    }

    pub(crate) fn into(self) -> ReplyItem {
        self.0
    }
}

impl FromRequest for Reply {
    fn from_request(self, _: HttpRequest) -> Reply {
        self
    }
}

impl FromRequest for HttpResponse {
    fn from_request(self, _: HttpRequest) -> Reply {
        Reply(ReplyItem::Message(self))
    }
}

#[cfg(actix_nightly)]
default impl<T: FromRequest> FromRequest for T
{
    fn from_request(self, req: HttpRequest) -> Reply {
        self.from_request(req)
    }
}

#[cfg(actix_nightly)]
default impl<T: FromRequest, E: Into<Error>> FromRequest for StdResult<T, E> {
    fn from_request(self, req: HttpRequest) -> Reply {
        match self {
            Ok(val) => val.from_request(req),
            Err(err) => Reply(ReplyItem::Message(err.into().into())),
        }
    }
}

impl<E: Into<Error>> FromRequest for StdResult<Reply, E> {
    fn from_request(self, _: HttpRequest) -> Reply {
        match self {
            Ok(val) => val,
            Err(err) => Reply(ReplyItem::Message(err.into().into())),
        }
    }
}

impl<E: Into<Error>> From<StdResult<Reply, E>> for Reply {
    fn from(res: StdResult<Reply, E>) -> Self {
        match res {
            Ok(val) => val,
            Err(err) => Reply(ReplyItem::Message(err.into().into())),
        }
    }
}

impl<E: Into<Error>> FromRequest for StdResult<HttpResponse, E> {
    fn from_request(self, _: HttpRequest) -> Reply {
        match self {
            Ok(val) => Reply(ReplyItem::Message(val)),
            Err(err) => Reply(ReplyItem::Message(err.into().into())),
        }
    }
}

impl<A: Actor<Context=HttpContext<A, S>>, S: 'static> FromRequest for HttpContext<A, S>
{
    fn from_request(self, _: HttpRequest) -> Reply {
        Reply(ReplyItem::Actor(Box::new(self)))
    }
}

impl<A: Actor<Context=HttpContext<A, S>>, S: 'static> From<HttpContext<A, S>> for Reply {
    fn from(ctx: HttpContext<A, S>) -> Reply {
        Reply(ReplyItem::Actor(Box::new(ctx)))
    }
}

impl FromRequest for Box<Future<Item=HttpResponse, Error=Error>>
{
    fn from_request(self, _: HttpRequest) -> Reply {
        Reply(ReplyItem::Future(self))
    }
}

/// Trait defines object that could be regestered as resource route
pub(crate) trait RouteHandler<S>: 'static {
    fn handle(&self, req: HttpRequest<S>) -> Reply;
}

/// Route handler wrapper for Handler
pub(crate)
struct WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: FromRequest,
          S: 'static,
{
    h: H,
    s: PhantomData<S>,
}

impl<S, H, R> WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: FromRequest,
          S: 'static,
{
    pub fn new(h: H) -> Self {
        WrapHandler{h: h, s: PhantomData}
    }
}

impl<S, H, R> RouteHandler<S> for WrapHandler<S, H, R>
    where H: Handler<S, Result=R>,
          R: FromRequest + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>) -> Reply {
        let req2 = req.clone_without_state();
        self.h.handle(req).from_request(req2)
    }
}

/// Async route handler
pub(crate)
struct AsyncHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Future<Item=HttpResponse, Error=Error> + 'static,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> AsyncHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Future<Item=HttpResponse, Error=Error> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        AsyncHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for AsyncHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Future<Item=HttpResponse, Error=Error> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>) -> Reply {
        Reply::async((self.f)(req))
    }
}

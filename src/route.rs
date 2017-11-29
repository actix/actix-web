use std::rc::Rc;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::result::Result as StdResult;

use actix::Actor;
// use http::{header, Version};
use futures::Stream;

use task::{Task, DrainFut, IoContext};
use body::Binary;
use error::{Error}; //, ExpectError, Result};
use context::HttpContext;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

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

/// Trait defines object that could be regestered as resource route
#[allow(unused_variables)]
pub trait RouteHandler<S>: 'static {
    /// Handle request
    fn handle(&self, req: HttpRequest<S>, task: &mut Task);

    /// Set route prefix
    fn set_prefix(&mut self, prefix: String) {}
}

/*
/// Actors with ability to handle http requests.
#[allow(unused_variables)]
pub trait RouteState {
    /// Shared state. State is shared with all routes within same application
    /// and could be accessed with `HttpRequest::state()` method.
    type State;

    /// Handle `EXPECT` header. By default respones with `HTTP/1.1 100 Continue`
    fn expect(req: &mut HttpRequest<Self::State>, ctx: &mut Self::Context) -> Result<()>
        where Self: Actor<Context=HttpContext<Self>>
    {
        // handle expect header only for HTTP/1.1
        if req.version() == Version::HTTP_11 {
            if let Some(expect) = req.headers().get(header::EXPECT) {
                if let Ok(expect) = expect.to_str() {
                    if expect.to_lowercase() == "100-continue" {
                        ctx.write("HTTP/1.1 100 Continue\r\n\r\n");
                        Ok(())
                    } else {
                        Err(ExpectError::UnknownExpect.into())
                    }
                } else {
                    Err(ExpectError::Encoding.into())
                }
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    /// Handle incoming request with http actor.
    fn handle(req: HttpRequest<Self::State>) -> Result<Reply>
        where Self: Default, Self: Actor<Context=HttpContext<Self>>
    {
        Ok(HttpContext::new(req, Self::default()).into())
    }
}*/

/// Fn() route handler
pub(crate)
struct FnHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Into<Reply>,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> FnHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Into<Reply> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        FnHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for FnHandler<S, R, F>
    where F: Fn(HttpRequest<S>) -> R + 'static,
          R: Into<Reply> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest<S>, task: &mut Task) {
        (self.f)(req).into().into(task)
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

enum ReplyItem {
    Message(HttpResponse),
    Actor(Box<IoContext<Item=Frame, Error=Error>>),
    Stream(Box<Stream<Item=Frame, Error=Error>>),
}

/// Represents response process.
pub struct Reply(ReplyItem);

impl Reply
{
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

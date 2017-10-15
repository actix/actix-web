use std::io;
use std::rc::Rc;
use std::marker::PhantomData;

use actix::Actor;
use bytes::Bytes;
use futures::Stream;

use task::Task;
use context::HttpContext;
use resource::Reply;
use payload::Payload;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

#[doc(hidden)]
#[derive(Debug)]
#[cfg_attr(feature="cargo-clippy", allow(large_enum_variant))]
pub enum Frame {
    Message(HttpResponse),
    Payload(Option<Bytes>),
}

/// Trait defines object that could be regestered as resource route
pub trait RouteHandler<S>: 'static {
    /// Handle request
    fn handle(&self, req: HttpRequest, payload: Payload, state: Rc<S>) -> Task;

    /// Set route prefix
    fn set_prefix(&mut self, _prefix: String) {}
}

/// Actors with ability to handle http requests
pub trait Route: Actor {
    /// Route shared state. State is shared with all routes within same application and could be
    /// accessed with `HttpContext::state()` method.
    type State;

    /// Handle incoming request. Route actor can return
    /// result immediately with `Reply::reply` or `Reply::with`.
    /// Actor itself could be returned for handling streaming request/response.
    /// In that case `HttpContext::start` and `HttpContext::write` has to be used.
    fn request(req: HttpRequest, payload: Payload, ctx: &mut Self::Context) -> Reply<Self>;

    /// This method creates `RouteFactory` for this actor.
    fn factory() -> RouteFactory<Self, Self::State> {
        RouteFactory(PhantomData)
    }
}

/// This is used for routes registration within `Resource`
pub struct RouteFactory<A: Route<State=S>, S>(PhantomData<A>);

impl<A, S> RouteHandler<S> for RouteFactory<A, S>
    where A: Actor<Context=HttpContext<A>> + Route<State=S>,
          S: 'static
{
    fn handle(&self, req: HttpRequest, payload: Payload, state: Rc<A::State>) -> Task
    {
        let mut ctx = HttpContext::new(state);
        A::request(req, payload, &mut ctx).into(ctx)
    }
}

/// Simple route handler
pub(crate)
struct FnHandler<S, R, F>
    where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
          R: Into<HttpResponse>,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> FnHandler<S, R, F>
    where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
          R: Into<HttpResponse> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        FnHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for FnHandler<S, R, F>
    where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
          R: Into<HttpResponse> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest, payload: Payload, state: Rc<S>) -> Task
    {
        Task::reply((self.f)(req, payload, &state).into())
    }
}

/// Async route handler
pub(crate)
struct StreamHandler<S, R, F>
    where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
          R: Stream<Item=Frame, Error=()> + 'static,
          S: 'static,
{
    f: Box<F>,
    s: PhantomData<S>,
}

impl<S, R, F> StreamHandler<S, R, F>
    where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
          R: Stream<Item=Frame, Error=()> + 'static,
          S: 'static,
{
    pub fn new(f: F) -> Self {
        StreamHandler{f: Box::new(f), s: PhantomData}
    }
}

impl<S, R, F> RouteHandler<S> for StreamHandler<S, R, F>
    where F: Fn(HttpRequest, Payload, &S) -> R + 'static,
          R: Stream<Item=Frame, Error=()> + 'static,
          S: 'static,
{
    fn handle(&self, req: HttpRequest, payload: Payload, state: Rc<S>) -> Task
    {
        Task::with_stream(
            (self.f)(req, payload, &state).map_err(
                |_| io::Error::new(io::ErrorKind::Other, ""))
        )
    }
}

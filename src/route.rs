use std::rc::Rc;
use std::marker::PhantomData;

use actix::Actor;
use bytes::Bytes;

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

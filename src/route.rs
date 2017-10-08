use std::rc::Rc;
use std::marker::PhantomData;

use actix::Actor;
use bytes::Bytes;
use futures::unsync::mpsc::Receiver;

use task::Task;
use context::HttpContext;
use resource::HttpMessage;
use httpmessage::{HttpRequest, HttpResponse};

/// Stream of `PayloadItem`'s
pub type Payload = Receiver<PayloadItem>;

/// `PayloadItem` represents one payload item
#[derive(Debug)]
pub enum PayloadItem {
    /// Indicates end of payload stream
    Eof,
    /// Chunk of bytes
    Chunk(Bytes)
}

impl PayloadItem {
    /// Is item an eof
    pub fn is_eof(&self) -> bool {
        match *self {
            PayloadItem::Eof => true,
            _ => false,
        }
    }
    /// Is item a chunk
    pub fn is_chunk(&self) -> bool {
        !self.is_eof()
    }
}


#[doc(hidden)]
#[derive(Debug)]
#[cfg_attr(feature="cargo-clippy", allow(large_enum_variant))]
pub enum Frame {
    Message(HttpResponse),
    Payload(Option<Bytes>),
}

/// Trait defines object that could be regestered as resource route.
pub trait RouteHandler<S>: 'static {
    fn handle(&self, req: HttpRequest, payload: Option<Payload>, state: Rc<S>) -> Task;
}

/// Actors with ability to handle http requests
pub trait Route: Actor<Context=HttpContext<Self>> {
    /// Route shared state. State is shared with all routes within same application and could be
    /// accessed with `HttpContext::state()` method.
    type State;

    /// Handle incoming request. Route actor can return
    /// result immediately with `HttpMessage::reply` or `HttpMessage::error`.
    /// Actor itself could be returned for handling streaming request/response.
    /// In that case `HttpContext::start` and `HttpContext::write` hs to be used.
    fn request(req: HttpRequest,
               payload: Option<Payload>,
               ctx: &mut HttpContext<Self>) -> HttpMessage<Self>;

    /// This method creates `RouteFactory` for this actor.
    fn factory() -> RouteFactory<Self, Self::State> {
        RouteFactory(PhantomData)
    }
}

/// This is used for routes registration within `HttpResource`.
pub struct RouteFactory<A: Route<State=S>, S>(PhantomData<A>);

impl<A, S> RouteHandler<S> for RouteFactory<A, S>
    where A: Route<State=S>,
          S: 'static
{
    fn handle(&self, req: HttpRequest, payload: Option<Payload>, state: Rc<A::State>) -> Task
    {
        let mut ctx = HttpContext::new(state);
        A::request(req, payload, &mut ctx).into(ctx)
    }
}

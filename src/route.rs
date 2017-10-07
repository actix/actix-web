use std;
use std::rc::Rc;
use std::marker::PhantomData;

use actix::Actor;
use bytes::Bytes;
use futures::unsync::mpsc::Receiver;

use task::Task;
use context::HttpContext;
use resource::HttpResponse;
use httpmessage::{HttpRequest, HttpMessage};

pub type Payload = Receiver<PayloadItem>;

#[derive(Debug)]
pub enum PayloadItem {
    Eof,
    Chunk(Bytes)
}

impl PayloadItem {
    pub fn is_eof(&self) -> bool {
        match *self {
            PayloadItem::Eof => true,
            _ => false,
        }
    }
    pub fn is_chunk(&self) -> bool {
        !self.is_eof()
    }
}


#[derive(Debug)]
#[cfg_attr(feature="cargo-clippy", allow(large_enum_variant))]
pub enum Frame {
    Message(HttpMessage),
    Payload(Option<Bytes>),
}

pub trait RouteHandler<S>: 'static {
    fn handle(&self, req: HttpRequest, payload: Option<Payload>, state: Rc<S>) -> Task;
}

pub trait Route: Actor<Context=HttpContext<Self>> {
    type State;

    fn request(req: HttpRequest,
               payload: Option<Payload>,
               ctx: &mut HttpContext<Self>) -> HttpResponse<Self>;

    fn factory() -> RouteFactory<Self, Self::State> {
        RouteFactory(PhantomData)
    }
}


pub struct RouteFactory<A: Route<State=S>, S>(PhantomData<A>);

impl<A, S> RouteHandler<S> for RouteFactory<A, S>
    where A: Route<State=S>,
          S: 'static
{
    fn handle(&self, req: HttpRequest, payload: Option<Payload>, state: Rc<A::State>) -> Task
    {
        let mut ctx = HttpContext::new(unsafe{std::mem::uninitialized()}, state);
        A::request(req, payload, &mut ctx).into(ctx)
    }
}

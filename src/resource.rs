use std::mem;
use std::rc::Rc;
use std::marker::PhantomData;
use std::collections::HashMap;

use actix::Actor;
use bytes::Bytes;
use http::Method;

use task::Task;
use route::{Route, Payload, RouteHandler};
use context::HttpContext;
use httpcodes::HTTPMethodNotAllowed;
use httpmessage::{HttpRequest, HttpMessage, IntoHttpMessage};

/// Resource
pub struct HttpResource<S=()> {
    state: PhantomData<S>,
    routes: HashMap<Method, Box<RouteHandler<S>>>,
    default: Box<RouteHandler<S>>,
}

impl<S> Default for HttpResource<S> {
    fn default() -> Self {
        HttpResource {
            state: PhantomData,
            routes: HashMap::new(),
            default: Box::new(HTTPMethodNotAllowed)}
    }
}


impl<S> HttpResource<S> where S: 'static {

    pub fn handler<H>(&mut self, method: Method, handler: H) -> &mut Self
        where H: RouteHandler<S>
    {
        self.routes.insert(method, Box::new(handler));
        self
    }

    pub fn default_handler<H>(&mut self, handler: H) -> &mut Self
        where H: RouteHandler<S>
    {
        self.default = Box::new(handler);
        self
    }

    pub fn get<A>(&mut self) -> &mut Self where A: Route<State=S>
    {
        self.handler(Method::GET, A::factory())
    }

    pub fn post<A>(&mut self) -> &mut Self where A: Route<State=S>
    {
        self.handler(Method::POST, A::factory())
    }

    pub fn put<A>(&mut self) -> &mut Self where A: Route<State=S>
    {
        self.handler(Method::PUT, A::factory())
    }

    pub fn delete<A>(&mut self) -> &mut Self where A: Route<State=S>
    {
        self.handler(Method::DELETE, A::factory())
    }
}


impl<S: 'static> RouteHandler<S> for HttpResource<S> {

    fn handle(&self, req: HttpRequest, payload: Option<Payload>, state: Rc<S>) -> Task {
        if let Some(handler) = self.routes.get(req.method()) {
            handler.handle(req, payload, state)
        } else {
            self.default.handle(req, payload, state)
        }
    }
}


#[cfg_attr(feature="cargo-clippy", allow(large_enum_variant))]
enum HttpResponseItem<A> where A: Actor<Context=HttpContext<A>> + Route {
    Message(HttpMessage, Option<Bytes>),
    Actor(A),
}

pub struct HttpResponse<A: Actor<Context=HttpContext<A>> + Route> (HttpResponseItem<A>);

impl<A> HttpResponse<A> where A: Actor<Context=HttpContext<A>> + Route
{
    /// Create async response
    #[allow(non_snake_case)]
    pub fn Stream(act: A) -> Self {
        HttpResponse(HttpResponseItem::Actor(act))
    }

    #[allow(non_snake_case)]
    pub fn Reply<I>(req: HttpRequest, msg: I) -> Self
        where I: IntoHttpMessage
    {
        HttpResponse(HttpResponseItem::Message(msg.into_response(req), None))
    }

    #[allow(non_snake_case)]
    pub fn ReplyMessage(msg: HttpMessage, body: Option<Bytes>) -> Self {
        HttpResponse(HttpResponseItem::Message(msg, body))
    }

    pub(crate) fn into(self, mut ctx: HttpContext<A>) -> Task {
        match self.0 {
            HttpResponseItem::Message(msg, body) =>
                Task::reply(msg, body),
            HttpResponseItem::Actor(act) => {
                let old = ctx.replace_actor(act);
                mem::forget(old);
                Task::with_stream(ctx)
            }
        }
    }
}

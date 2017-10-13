use std::rc::Rc;
use std::convert::From;
use std::marker::PhantomData;
use std::collections::HashMap;

use actix::Actor;
use http::Method;

use task::Task;
use route::{Route, RouteHandler};
use payload::Payload;
use context::HttpContext;
use httpcodes::HTTPMethodNotAllowed;
use httpmessage::{HttpRequest, HttpResponse};

/// Http resource
///
/// `Resource` is an entry in route table which corresponds to requested URL.
///
/// Resource in turn has at least one route.
/// Route corresponds to handling HTTP method by calling route handler.
///
/// ```rust,ignore
///
/// struct MyRoute;
///
/// fn main() {
///     let mut routes = RoutingMap::default();
///
///     routes
///      .add_resource("/")
///         .post::<MyRoute>();
/// }
pub struct Resource<S=()> {
    state: PhantomData<S>,
    routes: HashMap<Method, Box<RouteHandler<S>>>,
    default: Box<RouteHandler<S>>,
}

impl<S> Default for Resource<S> {
    fn default() -> Self {
        Resource {
            state: PhantomData,
            routes: HashMap::new(),
            default: Box::new(HTTPMethodNotAllowed)}
    }
}


impl<S> Resource<S> where S: 'static {

    /// Register handler for specified method.
    pub fn handler<H>(&mut self, method: Method, handler: H) -> &mut Self
        where H: RouteHandler<S>
    {
        self.routes.insert(method, Box::new(handler));
        self
    }

    /// Default handler is used if no matched route found.
    /// By default `HTTPMethodNotAllowed` is used.
    pub fn default_handler<H>(&mut self, handler: H) -> &mut Self
        where H: RouteHandler<S>
    {
        self.default = Box::new(handler);
        self
    }

    /// Handler for `GET` method.
    pub fn get<A>(&mut self) -> &mut Self
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.handler(Method::GET, A::factory())
    }

    /// Handler for `POST` method.
    pub fn post<A>(&mut self) -> &mut Self
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.handler(Method::POST, A::factory())
    }

    /// Handler for `PUR` method.
    pub fn put<A>(&mut self) -> &mut Self
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.handler(Method::PUT, A::factory())
    }

    /// Handler for `METHOD` method.
    pub fn delete<A>(&mut self) -> &mut Self
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.handler(Method::DELETE, A::factory())
    }
}


impl<S: 'static> RouteHandler<S> for Resource<S> {

    fn handle(&self, req: HttpRequest, payload: Payload, state: Rc<S>) -> Task {
        if let Some(handler) = self.routes.get(req.method()) {
            handler.handle(req, payload, state)
        } else {
            self.default.handle(req, payload, state)
        }
    }
}


#[cfg_attr(feature="cargo-clippy", allow(large_enum_variant))]
enum ReplyItem<A> where A: Actor + Route {
    Message(HttpResponse),
    Actor(A),
}

/// Represents response process.
pub struct Reply<A: Actor + Route> (ReplyItem<A>);

impl<A> Reply<A> where A: Actor + Route
{
    /// Create async response
    pub fn stream(act: A) -> Self {
        Reply(ReplyItem::Actor(act))
    }

    /// Send response
    pub fn reply<R: Into<HttpResponse>>(response: R) -> Self {
        Reply(ReplyItem::Message(response.into()))
    }

    pub fn into(self, mut ctx: HttpContext<A>) -> Task where A: Actor<Context=HttpContext<A>>
    {
        match self.0 {
            ReplyItem::Message(msg) => {
                Task::reply(msg)
            },
            ReplyItem::Actor(act) => {
                ctx.set_actor(act);
                Task::with_stream(ctx)
            }
        }
    }
}

impl<A, T> From<T> for Reply<A>
    where T: Into<HttpResponse>, A: Actor + Route
{
    fn from(item: T) -> Self {
        Reply::reply(item)
    }
}

#[cfg(feature="nightly")]
use std::ops::Try;

#[cfg(feature="nightly")]
impl<A> Try for Reply<A> where A: Actor + Route {
    type Ok = HttpResponse;
    type Error = HttpResponse;

    fn into_result(self) -> Result<Self::Ok, Self::Error> {
        panic!("Reply -> Result conversion is not supported")
    }

    fn from_error(v: Self::Error) -> Self {
        Reply::reply(v)
    }

    fn from_ok(v: Self::Ok) -> Self {
        Reply::reply(v)
    }
}

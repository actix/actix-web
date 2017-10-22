use std::rc::Rc;
use std::convert::From;
use std::marker::PhantomData;
use std::collections::HashMap;

use actix::Actor;
use http::Method;
use futures::Stream;

use task::Task;
use route::{Route, RouteHandler, RouteResult, Frame, FnHandler, StreamHandler};
use payload::Payload;
use context::HttpContext;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use httpcodes::HTTPMethodNotAllowed;

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
///     let router = RoutingMap::default()
///         .resource("/", |r| r.post::<MyRoute>())
///         .finish();
/// }
pub struct Resource<S=()> {
    name: String,
    state: PhantomData<S>,
    routes: HashMap<Method, Box<RouteHandler<S>>>,
    default: Box<RouteHandler<S>>,
}

impl<S> Default for Resource<S> {
    fn default() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: HashMap::new(),
            default: Box::new(HTTPMethodNotAllowed)}
    }
}


impl<S> Resource<S> where S: 'static {

    /// Set resource name
    pub fn set_name<T: ToString>(&mut self, name: T) {
        self.name = name.to_string();
    }

    /// Register handler for specified method.
    pub fn handler<F, R>(&mut self, method: Method, handler: F)
        where F: Fn(&mut HttpRequest, Payload, &S) -> R + 'static,
              R: Into<HttpResponse> + 'static,
    {
        self.routes.insert(method, Box::new(FnHandler::new(handler)));
    }

    /// Register async handler for specified method.
    pub fn async<F, R>(&mut self, method: Method, handler: F)
        where F: Fn(&mut HttpRequest, Payload, &S) -> R + 'static,
              R: Stream<Item=Frame, Error=()> + 'static,
    {
        self.routes.insert(method, Box::new(StreamHandler::new(handler)));
    }

    /// Register handler for specified method.
    pub fn route_handler<H>(&mut self, method: Method, handler: H)
        where H: RouteHandler<S>
    {
        self.routes.insert(method, Box::new(handler));
    }

    /// Default handler is used if no matched route found.
    /// By default `HTTPMethodNotAllowed` is used.
    pub fn default_handler<H>(&mut self, handler: H)
        where H: RouteHandler<S>
    {
        self.default = Box::new(handler);
    }

    /// Handler for `GET` method.
    pub fn get<A>(&mut self)
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.route_handler(Method::GET, A::factory());
    }

    /// Handler for `POST` method.
    pub fn post<A>(&mut self)
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.route_handler(Method::POST, A::factory());
    }

    /// Handler for `PUR` method.
    pub fn put<A>(&mut self)
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.route_handler(Method::PUT, A::factory());
    }

    /// Handler for `METHOD` method.
    pub fn delete<A>(&mut self)
        where A: Actor<Context=HttpContext<A>> + Route<State=S>
    {
        self.route_handler(Method::DELETE, A::factory());
    }
}


impl<S: 'static> RouteHandler<S> for Resource<S> {

    fn handle(&self, req: &mut HttpRequest, payload: Payload, state: Rc<S>) -> Task {
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
    pub fn async(act: A) -> RouteResult<A> {
        Ok(Reply(ReplyItem::Actor(act)))
    }

    /// Send response
    pub fn reply<R: Into<HttpResponse>>(response: R) -> RouteResult<A> {
        Ok(Reply(ReplyItem::Message(response.into())))
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
        Reply(ReplyItem::Message(item.into()))
    }
}

use std::marker::PhantomData;
use std::collections::HashMap;

use http::Method;
use futures::Stream;

use task::Task;
use error::Error;
use route::{Reply, RouteHandler, Frame, FnHandler, StreamHandler};
use httprequest::HttpRequest;
use httpcodes::{HTTPNotFound, HTTPMethodNotAllowed};

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

    pub(crate) fn default_not_found() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: HashMap::new(),
            default: Box::new(HTTPNotFound)}
    }

    /// Set resource name
    pub fn set_name<T: Into<String>>(&mut self, name: T) {
        self.name = name.into();
    }

    /// Register handler for specified method.
    pub fn handler<F, R>(&mut self, method: Method, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Into<Reply> + 'static,
    {
        self.routes.insert(method, Box::new(FnHandler::new(handler)));
    }

    /// Register async handler for specified method.
    pub fn async<F, R>(&mut self, method: Method, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Stream<Item=Frame, Error=Error> + 'static,
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
    pub fn get<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Into<Reply> + 'static, {
        self.routes.insert(Method::GET, Box::new(FnHandler::new(handler)));
    }

    /// Handler for `POST` method.
    pub fn post<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Into<Reply> + 'static, {
        self.routes.insert(Method::POST, Box::new(FnHandler::new(handler)));
    }

    /// Handler for `PUT` method.
    pub fn put<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Into<Reply> + 'static, {
        self.routes.insert(Method::PUT, Box::new(FnHandler::new(handler)));
    }

    /// Handler for `DELETE` method.
    pub fn delete<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Into<Reply> + 'static, {
        self.routes.insert(Method::DELETE, Box::new(FnHandler::new(handler)));
    }
}


impl<S: 'static> RouteHandler<S> for Resource<S> {

    fn handle(&self, req: HttpRequest<S>, task: &mut Task) {
        if let Some(handler) = self.routes.get(req.method()) {
            handler.handle(req, task)
        } else {
            self.default.handle(req, task)
        }
    }
}

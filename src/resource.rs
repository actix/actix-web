use std::marker::PhantomData;

use http::Method;

use route::Route;
use handler::{Reply, Handler, RouteHandler, WrapHandler};
use httpcodes::HTTPNotFound;
use httprequest::HttpRequest;

/// *Resource* is an entry in route table which corresponds to requested URL.
///
/// Resource in turn has at least one route.
/// Route consists of an object that implements `Handler` trait (handler)
/// and list of predicates (objects that implement `Predicate` trait).
/// Route uses builder-like pattern for configuration.
/// During request handling, resource object iterate through all routes
/// and check all predicates for specific route, if request matches all predicates route
/// route considired matched and route handler get called.
///
/// ```rust
/// extern crate actix_web;
/// use actix_web::*;
///
/// fn main() {
///     let app = Application::default("/")
///         .resource(
///             "/", |r| r.route().method(Method::GET).f(|r| HttpResponse::Ok()))
///         .finish();
/// }
pub struct Resource<S=()> {
    name: String,
    state: PhantomData<S>,
    routes: Vec<Route<S>>,
    default: Box<RouteHandler<S>>,
}

impl<S> Default for Resource<S> {
    fn default() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: Vec::new(),
            default: Box::new(HTTPNotFound)}
    }
}

impl<S> Resource<S> where S: 'static {

    pub(crate) fn default_not_found() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: Vec::new(),
            default: Box::new(HTTPNotFound)}
    }

    /// Set resource name
    pub fn name<T: Into<String>>(&mut self, name: T) {
        self.name = name.into();
    }

    /// Register a new route and return mutable reference to *Route* object.
    /// *Route* is used for route configuration, i.e. adding predicates, setting up handler.
    ///
    /// ```rust
    /// extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = Application::default("/")
    ///         .resource(
    ///             "/", |r| r.route()
    ///                  .p(pred::Any(vec![pred::Get(), pred::Put()]))
    ///                  .p(pred::Header("Content-Type", "text/plain"))
    ///                  .f(|r| HttpResponse::Ok()))
    ///         .finish();
    /// }
    pub fn route(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap()
    }

    /// Register a new route and add method check to route.
    pub fn method(&mut self, method: Method) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().method(method)
    }

    /// Default handler is used if no matched route found.
    /// By default `HTTPNotFound` is used.
    pub fn default_handler<H>(&mut self, handler: H) where H: Handler<S> {
        self.default = Box::new(WrapHandler::new(handler));
    }
}

impl<S: 'static> RouteHandler<S> for Resource<S> {

    fn handle(&self, mut req: HttpRequest<S>) -> Reply {
        for route in &self.routes {
            if route.check(&mut req) {
                return route.handle(req)
            }
        }
        self.default.handle(req)
    }
}

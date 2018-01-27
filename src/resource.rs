use std::rc::Rc;
use std::marker::PhantomData;

use http::{Method, StatusCode};

use pred;
use body::Body;
use route::Route;
use handler::{Reply, Handler, Responder};
use middleware::Middleware;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// *Resource* is an entry in route table which corresponds to requested URL.
///
/// Resource in turn has at least one route.
/// Route consists of an object that implements `Handler` trait (handler)
/// and list of predicates (objects that implement `Predicate` trait).
/// Route uses builder-like pattern for configuration.
/// During request handling, resource object iterate through all routes
/// and check all predicates for specific route, if request matches all predicates route
/// route considered matched and route handler get called.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::*;
///
/// fn main() {
///     let app = Application::new()
///         .resource(
///             "/", |r| r.method(Method::GET).f(|r| HttpResponse::Ok()))
///         .finish();
/// }
pub struct Resource<S=()> {
    name: String,
    state: PhantomData<S>,
    routes: Vec<Route<S>>,
    middlewares: Rc<Vec<Box<Middleware<S>>>>,
}

impl<S> Default for Resource<S> {
    fn default() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: Vec::new(),
            middlewares: Rc::new(Vec::new()) }
    }
}

impl<S> Resource<S> {

    pub(crate) fn default_not_found() -> Self {
        Resource {
            name: String::new(),
            state: PhantomData,
            routes: Vec::new(),
            middlewares: Rc::new(Vec::new()) }
    }

    /// Set resource name
    pub fn name<T: Into<String>>(&mut self, name: T) {
        self.name = name.into();
    }

    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }
}

impl<S: 'static> Resource<S> {

    /// Register a new route and return mutable reference to *Route* object.
    /// *Route* is used for route configuration, i.e. adding predicates, setting up handler.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = Application::new()
    ///         .resource(
    ///             "/", |r| r.route()
    ///                  .p(pred::Any(pred::Get()).or(pred::Put()))
    ///                  .p(pred::Header("Content-Type", "text/plain"))
    ///                  .f(|r| HttpResponse::Ok()))
    ///         .finish();
    /// }
    /// ```
    pub fn route(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap()
    }

    /// Register a new route and add method check to route.
    ///
    /// This is shortcut for:
    ///
    /// ```rust,ignore
    /// Resource::resource("/", |r| r.route().p(pred::Get()).f(index)
    /// ```
    pub fn method(&mut self, method: Method) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().p(pred::Method(method))
    }

    /// Register a new route and add handler object.
    ///
    /// This is shortcut for:
    ///
    /// ```rust,ignore
    /// Resource::resource("/", |r| r.route().h(handler)
    /// ```
    pub fn h<H: Handler<S>>(&mut self, handler: H) {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().h(handler)
    }

    /// Register a new route and add handler function.
    ///
    /// This is shortcut for:
    ///
    /// ```rust,ignore
    /// Resource::resource("/", |r| r.route().f(index)
    /// ```
    pub fn f<F, R>(&mut self, handler: F)
        where F: Fn(HttpRequest<S>) -> R + 'static,
              R: Responder + 'static,
    {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().f(handler)
    }

    /// Register a middleware
    ///
    /// This is similar to `Application's` middlewares, but
    /// middlewares get invoked on resource level.
    pub fn middleware<M: Middleware<S>>(&mut self, mw: M) {
        Rc::get_mut(&mut self.middlewares).unwrap().push(Box::new(mw));
    }

    pub(crate) fn handle(&mut self,
                         mut req: HttpRequest<S>,
                         default: Option<&mut Resource<S>>) -> Reply
    {
        for route in &mut self.routes {
            if route.check(&mut req) {
                return if self.middlewares.is_empty() {
                    route.handle(req)
                } else {
                    route.compose(req, Rc::clone(&self.middlewares))
                };
            }
        }
        if let Some(resource) = default {
            resource.handle(req, None)
        } else {
            Reply::response(HttpResponse::new(StatusCode::NOT_FOUND, Body::Empty))
        }
    }
}

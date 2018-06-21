use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use futures::Future;
use http::Method;
use smallvec::SmallVec;

use error::Error;
use handler::{AsyncResult, FromRequest, Handler, Responder};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::Middleware;
use pred;
use route::Route;

/// *Resource* is an entry in route table which corresponds to requested URL.
///
/// Resource in turn has at least one route.
/// Route consists of an object that implements `Handler` trait (handler)
/// and list of predicates (objects that implement `Predicate` trait).
/// Route uses builder-like pattern for configuration.
/// During request handling, resource object iterate through all routes
/// and check all predicates for specific route, if request matches all
/// predicates route route considered matched and route handler get called.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{App, HttpResponse, http};
///
/// fn main() {
///     let app = App::new()
///         .resource(
///             "/", |r| r.method(http::Method::GET).f(|r| HttpResponse::Ok()))
///         .finish();
/// }
pub struct ResourceHandler<S = ()> {
    name: String,
    state: PhantomData<S>,
    routes: SmallVec<[Route<S>; 3]>,
    middlewares: Rc<RefCell<Vec<Box<Middleware<S>>>>>,
}

impl<S> Default for ResourceHandler<S> {
    fn default() -> Self {
        ResourceHandler {
            name: String::new(),
            state: PhantomData,
            routes: SmallVec::new(),
            middlewares: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl<S> ResourceHandler<S> {
    pub(crate) fn default_not_found() -> Self {
        ResourceHandler {
            name: String::new(),
            state: PhantomData,
            routes: SmallVec::new(),
            middlewares: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Set resource name
    pub fn name<T: Into<String>>(&mut self, name: T) {
        self.name = name.into();
    }

    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }
}

impl<S: 'static> ResourceHandler<S> {
    /// Register a new route and return mutable reference to *Route* object.
    /// *Route* is used for route configuration, i.e. adding predicates,
    /// setting up handler.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .resource("/", |r| {
    ///             r.route()
    ///                 .filter(pred::Any(pred::Get()).or(pred::Put()))
    ///                 .filter(pred::Header("Content-Type", "text/plain"))
    ///                 .f(|r| HttpResponse::Ok())
    ///         })
    ///         .finish();
    /// }
    /// ```
    pub fn route(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap()
    }

    /// Register a new `GET` route.
    pub fn get(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().filter(pred::Get())
    }

    /// Register a new `POST` route.
    pub fn post(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().filter(pred::Post())
    }

    /// Register a new `PUT` route.
    pub fn put(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().filter(pred::Put())
    }

    /// Register a new `DELETE` route.
    pub fn delete(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().filter(pred::Delete())
    }

    /// Register a new `HEAD` route.
    pub fn head(&mut self) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().filter(pred::Head())
    }

    /// Register a new route and add method check to route.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    /// fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    ///
    /// App::new().resource("/", |r| r.method(http::Method::GET).f(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    /// App::new().resource("/", |r| r.route().filter(pred::Get()).f(index));
    /// ```
    pub fn method(&mut self, method: Method) -> &mut Route<S> {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().filter(pred::Method(method))
    }

    /// Register a new route and add handler object.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    /// fn handler(req: HttpRequest) -> HttpResponse { unimplemented!() }
    ///
    /// App::new().resource("/", |r| r.h(handler));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn handler(req: HttpRequest) -> HttpResponse { unimplemented!() }
    /// App::new().resource("/", |r| r.route().h(handler));
    /// ```
    pub fn h<H: Handler<S>>(&mut self, handler: H) {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().h(handler)
    }

    /// Register a new route and add handler function.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    /// fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    ///
    /// App::new().resource("/", |r| r.f(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    /// App::new().resource("/", |r| r.route().f(index));
    /// ```
    pub fn f<F, R>(&mut self, handler: F)
    where
        F: Fn(HttpRequest<S>) -> R + 'static,
        R: Responder + 'static,
    {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().f(handler)
    }

    /// Register a new route and add handler.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::*;
    /// fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    ///
    /// App::new().resource("/", |r| r.with(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    /// App::new().resource("/", |r| r.route().with(index));
    /// ```
    pub fn with<T, F, R>(&mut self, handler: F)
    where
        F: Fn(T) -> R + 'static,
        R: Responder + 'static,
        T: FromRequest<S> + 'static,
    {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().with(handler);
    }

    /// Register a new route and add async handler.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// use actix_web::*;
    /// use futures::future::Future;
    ///
    /// fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    ///     unimplemented!()
    /// }
    ///
    /// App::new().resource("/", |r| r.with_async(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # use actix_web::*;
    /// # use futures::future::Future;
    /// # fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    /// #     unimplemented!()
    /// # }
    /// App::new().resource("/", |r| r.route().with_async(index));
    /// ```
    pub fn with_async<T, F, R, I, E>(&mut self, handler: F)
    where
        F: Fn(T) -> R + 'static,
        R: Future<Item = I, Error = E> + 'static,
        I: Responder + 'static,
        E: Into<Error> + 'static,
        T: FromRequest<S> + 'static,
    {
        self.routes.push(Route::default());
        self.routes.last_mut().unwrap().with_async(handler);
    }

    /// Register a resource middleware
    ///
    /// This is similar to `App's` middlewares, but
    /// middlewares get invoked on resource level.
    ///
    /// *Note* `Middleware::finish()` fires right after response get
    /// prepared. It does not wait until body get sent to peer.
    pub fn middleware<M: Middleware<S>>(&mut self, mw: M) {
        Rc::get_mut(&mut self.middlewares)
            .unwrap()
            .borrow_mut()
            .push(Box::new(mw));
    }

    pub(crate) fn handle(
        &self, mut req: HttpRequest<S>,
    ) -> Result<AsyncResult<HttpResponse>, HttpRequest<S>> {
        for route in &self.routes {
            if route.check(&mut req) {
                return if self.middlewares.borrow().is_empty() {
                    Ok(route.handle(req))
                } else {
                    Ok(route.compose(req, Rc::clone(&self.middlewares)))
                };
            }
        }

        Err(req)
    }
}

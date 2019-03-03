use std::cell::RefCell;
use std::rc::Rc;

use actix_http::{http::Method, Error, Extensions, Response};
use actix_service::{NewService, Service};
use futures::{Async, Future, IntoFuture, Poll};

use crate::guard::{self, Guard};
use crate::handler::{
    AsyncFactory, AsyncHandle, ConfigStorage, Extract, ExtractorConfig, Factory,
    FromRequest, Handle,
};
use crate::responder::Responder;
use crate::service::{ServiceFromRequest, ServiceRequest, ServiceResponse};
use crate::HttpResponse;

type BoxedRouteService<Req, Res> = Box<
    Service<
        Request = Req,
        Response = Res,
        Error = (),
        Future = Box<Future<Item = Res, Error = ()>>,
    >,
>;

type BoxedRouteNewService<Req, Res> = Box<
    NewService<
        Request = Req,
        Response = Res,
        Error = (),
        InitError = (),
        Service = BoxedRouteService<Req, Res>,
        Future = Box<Future<Item = BoxedRouteService<Req, Res>, Error = ()>>,
    >,
>;

/// Resource route definition
///
/// Route uses builder-like pattern for configuration.
/// If handler is not explicitly set, default *404 Not Found* handler is used.
pub struct Route<P> {
    service: BoxedRouteNewService<ServiceRequest<P>, ServiceResponse>,
    guards: Rc<Vec<Box<Guard>>>,
    config: ConfigStorage,
    config_ref: Rc<RefCell<Option<Rc<Extensions>>>>,
}

impl<P: 'static> Route<P> {
    /// Create new route which matches any request.
    pub fn new() -> Route<P> {
        let config_ref = Rc::new(RefCell::new(None));
        Route {
            service: Box::new(RouteNewService::new(
                Extract::new(config_ref.clone()).and_then(
                    Handle::new(|| HttpResponse::NotFound()).map_err(|_| panic!()),
                ),
            )),
            guards: Rc::new(Vec::new()),
            config: ConfigStorage::default(),
            config_ref,
        }
    }

    /// Create new `GET` route.
    pub fn get() -> Route<P> {
        Route::new().method(Method::GET)
    }

    /// Create new `POST` route.
    pub fn post() -> Route<P> {
        Route::new().method(Method::POST)
    }

    /// Create new `PUT` route.
    pub fn put() -> Route<P> {
        Route::new().method(Method::PUT)
    }

    /// Create new `DELETE` route.
    pub fn delete() -> Route<P> {
        Route::new().method(Method::DELETE)
    }

    pub(crate) fn finish(self) -> Self {
        *self.config_ref.borrow_mut() = self.config.storage.clone();
        self
    }
}

impl<P> NewService for Route<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = RouteService<P>;
    type Future = CreateRouteService<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        CreateRouteService {
            fut: self.service.new_service(&()),
            guards: self.guards.clone(),
        }
    }
}

type RouteFuture<P> = Box<
    Future<Item = BoxedRouteService<ServiceRequest<P>, ServiceResponse>, Error = ()>,
>;

pub struct CreateRouteService<P> {
    fut: RouteFuture<P>,
    guards: Rc<Vec<Box<Guard>>>,
}

impl<P> Future for CreateRouteService<P> {
    type Item = RouteService<P>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(service) => Ok(Async::Ready(RouteService {
                service,
                guards: self.guards.clone(),
            })),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

pub struct RouteService<P> {
    service: BoxedRouteService<ServiceRequest<P>, ServiceResponse>,
    guards: Rc<Vec<Box<Guard>>>,
}

impl<P> RouteService<P> {
    pub fn check(&self, req: &mut ServiceRequest<P>) -> bool {
        for f in self.guards.iter() {
            if !f.check(req.head()) {
                return false;
            }
        }
        true
    }
}

impl<P> Service for RouteService<P> {
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.service.call(req)
    }
}

impl<P: 'static> Route<P> {
    /// Add method guard to the route.
    ///
    /// ```rust
    /// # use actix_web::*;
    /// # fn main() {
    /// App::new().resource("/path", |r| {
    ///     r.route(web::get()
    ///         .guard(guard::Get())
    ///         .guard(guard::Header("content-type", "text/plain"))
    ///         .to(|req: HttpRequest| HttpResponse::Ok()))
    /// });
    /// # }
    /// ```
    pub fn method(mut self, method: Method) -> Self {
        Rc::get_mut(&mut self.guards)
            .unwrap()
            .push(Box::new(guard::Method(method)));
        self
    }

    /// Add guard to the route.
    ///
    /// ```rust
    /// # use actix_web::*;
    /// # fn main() {
    /// App::new().resource("/path", |r| {
    ///     r.route(web::route()
    ///         .guard(guard::Get())
    ///         .guard(guard::Header("content-type", "text/plain"))
    ///         .to(|req: HttpRequest| HttpResponse::Ok()))
    /// });
    /// # }
    /// ```
    pub fn guard<F: Guard + 'static>(mut self, f: F) -> Self {
        Rc::get_mut(&mut self.guards).unwrap().push(Box::new(f));
        self
    }

    // pub fn map<T, U, F: IntoNewService<T>>(
    //     self,
    //     md: F,
    // ) -> RouteServiceBuilder<T, S, (), U>
    // where
    //     T: NewService<
    //         Request = HandlerRequest<S>,
    //         Response = HandlerRequest<S, U>,
    //         InitError = (),
    //     >,
    // {
    //     RouteServiceBuilder {
    //         service: md.into_new_service(),
    //         guards: self.guards,
    //         _t: PhantomData,
    //     }
    // }

    /// Set handler function, use request extractors for parameters.
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_derive;
    /// use actix_web::{web, http, App, Path};
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// fn index(info: Path<Info>) -> String {
    ///     format!("Welcome {}!", info.username)
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().resource(
    ///         "/{username}/index.html", // <- define path parameters
    ///         |r| r.route(web::get().to(index)), // <- register handler
    ///     );
    /// }
    /// ```
    ///
    /// It is possible to use multiple extractors for one handler function.
    ///
    /// ```rust
    /// # use std::collections::HashMap;
    /// # use serde_derive::Deserialize;
    /// use actix_web::{web, http, App, Json, Path, Query};
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// fn index(path: Path<Info>, query: Query<HashMap<String, String>>, body: Json<Info>) -> String {
    ///     format!("Welcome {}!", path.username)
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().resource(
    ///         "/{username}/index.html", // <- define path parameters
    ///         |r| r.route(web::method(http::Method::GET).to(index)),
    ///     );
    /// }
    /// ```
    pub fn to<F, T, R>(mut self, handler: F) -> Route<P>
    where
        F: Factory<T, R> + 'static,
        T: FromRequest<P> + 'static,
        R: Responder + 'static,
    {
        T::Config::store_default(&mut self.config);
        self.service = Box::new(RouteNewService::new(
            Extract::new(self.config_ref.clone())
                .and_then(Handle::new(handler).map_err(|_| panic!())),
        ));
        self
    }

    /// Set async handler function, use request extractors for parameters.
    /// This method has to be used if your handler function returns `impl Future<>`
    ///
    /// ```rust
    /// # use futures::future::ok;
    /// #[macro_use] extern crate serde_derive;
    /// use actix_web::{web, http, App, Error, Path};
    /// use futures::Future;
    ///
    /// #[derive(Deserialize)]
    /// struct Info {
    ///     username: String,
    /// }
    ///
    /// /// extract path info using serde
    /// fn index(info: Path<Info>) -> impl Future<Item = &'static str, Error = Error> {
    ///     ok("Hello World!")
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().resource(
    ///         "/{username}/index.html", // <- define path parameters
    ///         |r| r.route(web::get().to_async(index)), // <- register async handler
    ///     );
    /// }
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn to_async<F, T, R>(mut self, handler: F) -> Self
    where
        F: AsyncFactory<T, R>,
        T: FromRequest<P> + 'static,
        R: IntoFuture + 'static,
        R::Item: Into<Response>,
        R::Error: Into<Error>,
    {
        self.service = Box::new(RouteNewService::new(
            Extract::new(self.config_ref.clone())
                .and_then(AsyncHandle::new(handler).map_err(|_| panic!())),
        ));
        self
    }

    /// This method allows to add extractor configuration
    /// for specific route.
    ///
    /// ```rust
    /// use actix_web::{web, extractor, App};
    ///
    /// /// extract text data from request
    /// fn index(body: String) -> String {
    ///     format!("Body {}!", body)
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().resource("/index.html", |r| {
    ///         r.route(
    ///             web::get()
    ///                // limit size of the payload
    ///                .config(extractor::PayloadConfig::new(4096))
    ///                // register handler
    ///                .to(index)
    ///         )
    ///     });
    /// }
    /// ```
    pub fn config<C: ExtractorConfig>(mut self, config: C) -> Self {
        self.config.store(config);
        self
    }
}

struct RouteNewService<P, T>
where
    T: NewService<Request = ServiceRequest<P>, Error = (Error, ServiceFromRequest<P>)>,
{
    service: T,
}

impl<P: 'static, T> RouteNewService<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (Error, ServiceFromRequest<P>),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    pub fn new(service: T) -> Self {
        RouteNewService { service }
    }
}

impl<P: 'static, T> NewService for RouteNewService<P, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (Error, ServiceFromRequest<P>),
    >,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service>::Future: 'static,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = BoxedRouteService<Self::Request, Self::Response>;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        Box::new(
            self.service
                .new_service(&())
                .map_err(|_| ())
                .and_then(|service| {
                    let service: BoxedRouteService<_, _> =
                        Box::new(RouteServiceWrapper { service });
                    Ok(service)
                }),
        )
    }
}

struct RouteServiceWrapper<P, T: Service<Request = ServiceRequest<P>>> {
    service: T,
}

impl<P, T> Service for RouteServiceWrapper<P, T>
where
    T::Future: 'static,
    T: Service<
        Request = ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (Error, ServiceFromRequest<P>),
    >,
{
    type Request = ServiceRequest<P>;
    type Response = ServiceResponse;
    type Error = ();
    type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, req: ServiceRequest<P>) -> Self::Future {
        Box::new(self.service.call(req).then(|res| match res {
            Ok(res) => Ok(res),
            Err((err, req)) => Ok(req.error_response(err)),
        }))
    }
}

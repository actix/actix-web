use std::cell::RefCell;
use std::rc::Rc;

use actix_http::{Error, Response};
use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{
    ApplyTransform, IntoNewService, IntoTransform, NewService, Service, Transform,
};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, IntoFuture, Poll};

use crate::dev::{insert_slash, AppConfig, HttpServiceFactory, ResourceDef};
use crate::extract::FromRequest;
use crate::guard::Guard;
use crate::handler::{AsyncFactory, Factory};
use crate::responder::Responder;
use crate::route::{CreateRouteService, Route, RouteService};
use crate::service::{ServiceRequest, ServiceResponse};

type HttpService<P> = BoxedService<ServiceRequest<P>, ServiceResponse, ()>;
type HttpNewService<P> = BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;

/// *Resource* is an entry in route table which corresponds to requested URL.
///
/// Resource in turn has at least one route.
/// Route consists of an handlers objects and list of guards
/// (objects that implement `Guard` trait).
/// Resources and rouets uses builder-like pattern for configuration.
/// During request handling, resource object iterate through all routes
/// and check guards for specific route, if request matches all
/// guards, route considered matched and route handler get called.
///
/// ```rust
/// use actix_web::{web, App, HttpResponse};
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/")
///             .route(web::get().to(|| HttpResponse::Ok())));
/// }
pub struct Resource<P, T = ResourceEndpoint<P>> {
    endpoint: T,
    rdef: String,
    routes: Vec<Route<P>>,
    guards: Vec<Box<Guard>>,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
    factory_ref: Rc<RefCell<Option<ResourceFactory<P>>>>,
}

impl<P> Resource<P> {
    pub fn new(path: &str) -> Resource<P> {
        let fref = Rc::new(RefCell::new(None));

        Resource {
            routes: Vec::new(),
            rdef: path.to_string(),
            endpoint: ResourceEndpoint::new(fref.clone()),
            factory_ref: fref,
            guards: Vec::new(),
            default: Rc::new(RefCell::new(None)),
        }
    }
}

impl<P, T> Resource<P, T>
where
    P: 'static,
    T: NewService<
        ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (),
        InitError = (),
    >,
{
    /// Add match guard to a resource.
    ///
    /// ```rust
    /// use actix_web::{web, guard, App, HttpResponse, extract::Path};
    ///
    /// fn index(data: Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .service(
    ///             web::resource("/app")
    ///                 .guard(guard::Header("content-type", "text/plain"))
    ///                 .route(web::get().to(index))
    ///         )
    ///         .service(
    ///             web::resource("/app")
    ///                 .guard(guard::Header("content-type", "text/json"))
    ///                 .route(web::get().to(|| HttpResponse::MethodNotAllowed()))
    ///         );
    /// }
    /// ```
    pub fn guard<G: Guard + 'static>(mut self, guard: G) -> Self {
        self.guards.push(Box::new(guard));
        self
    }

    pub(crate) fn add_guards(mut self, guards: Vec<Box<Guard>>) -> Self {
        self.guards.extend(guards);
        self
    }

    /// Register a new route.
    ///
    /// ```rust
    /// use actix_web::{web, guard, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/").route(
    ///             web::route()
    ///                 .guard(guard::Any(guard::Get()).or(guard::Put()))
    ///                 .guard(guard::Header("Content-Type", "text/plain"))
    ///                 .to(|| HttpResponse::Ok()))
    ///     );
    /// }
    /// ```
    ///
    /// Multiple routes could be added to a resource.
    ///
    /// ```rust
    /// use actix_web::{web, guard, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/container/")
    ///              .route(web::get().to(get_handler))
    ///              .route(web::post().to(post_handler))
    ///              .route(web::delete().to(delete_handler))
    ///     );
    /// }
    /// # fn get_handler() {}
    /// # fn post_handler() {}
    /// # fn delete_handler() {}
    /// ```
    pub fn route(mut self, route: Route<P>) -> Self {
        self.routes.push(route.finish());
        self
    }

    /// Register a new route and add handler. This route matches all requests.
    ///
    /// ```rust
    /// use actix_web::*;
    ///
    /// fn index(req: HttpRequest) -> HttpResponse {
    ///     unimplemented!()
    /// }
    ///
    /// App::new().service(web::resource("/").to(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    /// App::new().service(web::resource("/").route(web::route().to(index)));
    /// ```
    pub fn to<F, I, R>(mut self, handler: F) -> Self
    where
        F: Factory<I, R> + 'static,
        I: FromRequest<P> + 'static,
        R: Responder + 'static,
    {
        self.routes.push(Route::new().to(handler));
        self
    }

    /// Register a new route and add async handler.
    ///
    /// ```rust
    /// use actix_web::*;
    /// use futures::future::{ok, Future};
    ///
    /// fn index(req: HttpRequest) -> impl Future<Item=HttpResponse, Error=Error> {
    ///     ok(HttpResponse::Ok().finish())
    /// }
    ///
    /// App::new().service(web::resource("/").to_async(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```rust
    /// # use actix_web::*;
    /// # use futures::future::Future;
    /// # fn index(req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
    /// #     unimplemented!()
    /// # }
    /// App::new().service(web::resource("/").route(web::route().to_async(index)));
    /// ```
    #[allow(clippy::wrong_self_convention)]
    pub fn to_async<F, I, R>(mut self, handler: F) -> Self
    where
        F: AsyncFactory<I, R>,
        I: FromRequest<P> + 'static,
        R: IntoFuture + 'static,
        R::Item: Into<Response>,
        R::Error: Into<Error>,
    {
        self.routes.push(Route::new().to_async(handler));
        self
    }

    /// Register a resource middleware
    ///
    /// This is similar to `App's` middlewares, but
    /// middleware is not allowed to change response type (i.e modify response's body).
    /// Middleware get invoked on resource level.
    pub fn middleware<M, F>(
        self,
        mw: F,
    ) -> Resource<
        P,
        impl NewService<
            ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        >,
        F: IntoTransform<M, T::Service, ServiceRequest<P>>,
    {
        let endpoint = ApplyTransform::new(mw, self.endpoint);
        Resource {
            endpoint,
            rdef: self.rdef,
            guards: self.guards,
            routes: self.routes,
            default: self.default,
            factory_ref: self.factory_ref,
        }
    }

    /// Default resource to be used if no matching route could be found.
    pub fn default_resource<F, R, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> R,
        R: IntoNewService<U, ServiceRequest<P>>,
        U: NewService<ServiceRequest<P>, Response = ServiceResponse, Error = ()>
            + 'static,
    {
        // create and configure default resource
        self.default = Rc::new(RefCell::new(Some(Rc::new(boxed::new_service(
            f(Resource::new("")).into_new_service().map_init_err(|_| ()),
        )))));

        self
    }
}

impl<P, T> HttpServiceFactory<P> for Resource<P, T>
where
    P: 'static,
    T: NewService<
            ServiceRequest<P>,
            Response = ServiceResponse,
            Error = (),
            InitError = (),
        > + 'static,
{
    fn register(mut self, config: &mut AppConfig<P>) {
        if self.default.borrow().is_none() {
            *self.default.borrow_mut() = Some(config.default_service());
        }
        let guards = if self.guards.is_empty() {
            None
        } else {
            Some(std::mem::replace(&mut self.guards, Vec::new()))
        };
        let rdef = if config.is_root() {
            ResourceDef::new(&insert_slash(&self.rdef))
        } else {
            ResourceDef::new(&insert_slash(&self.rdef))
        };
        config.register_service(rdef, guards, self)
    }
}

impl<P, T> IntoNewService<T, ServiceRequest<P>> for Resource<P, T>
where
    T: NewService<
        ServiceRequest<P>,
        Response = ServiceResponse,
        Error = (),
        InitError = (),
    >,
{
    fn into_new_service(self) -> T {
        *self.factory_ref.borrow_mut() = Some(ResourceFactory {
            routes: self.routes,
            default: self.default,
        });

        self.endpoint
    }
}

pub struct ResourceFactory<P> {
    routes: Vec<Route<P>>,
    default: Rc<RefCell<Option<Rc<HttpNewService<P>>>>>,
}

impl<P: 'static> NewService<ServiceRequest<P>> for ResourceFactory<P> {
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = ResourceService<P>;
    type Future = CreateResourceService<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        let default_fut = if let Some(ref default) = *self.default.borrow() {
            Some(default.new_service(&()))
        } else {
            None
        };

        CreateResourceService {
            fut: self
                .routes
                .iter()
                .map(|route| CreateRouteServiceItem::Future(route.new_service(&())))
                .collect(),
            default: None,
            default_fut,
        }
    }
}

enum CreateRouteServiceItem<P> {
    Future(CreateRouteService<P>),
    Service(RouteService<P>),
}

pub struct CreateResourceService<P> {
    fut: Vec<CreateRouteServiceItem<P>>,
    default: Option<HttpService<P>>,
    default_fut: Option<Box<Future<Item = HttpService<P>, Error = ()>>>,
}

impl<P> Future for CreateResourceService<P> {
    type Item = ResourceService<P>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut done = true;

        if let Some(ref mut fut) = self.default_fut {
            match fut.poll()? {
                Async::Ready(default) => self.default = Some(default),
                Async::NotReady => done = false,
            }
        }

        // poll http services
        for item in &mut self.fut {
            match item {
                CreateRouteServiceItem::Future(ref mut fut) => match fut.poll()? {
                    Async::Ready(route) => {
                        *item = CreateRouteServiceItem::Service(route)
                    }
                    Async::NotReady => {
                        done = false;
                    }
                },
                CreateRouteServiceItem::Service(_) => continue,
            };
        }

        if done {
            let routes = self
                .fut
                .drain(..)
                .map(|item| match item {
                    CreateRouteServiceItem::Service(service) => service,
                    CreateRouteServiceItem::Future(_) => unreachable!(),
                })
                .collect();
            Ok(Async::Ready(ResourceService {
                routes,
                default: self.default.take(),
            }))
        } else {
            Ok(Async::NotReady)
        }
    }
}

pub struct ResourceService<P> {
    routes: Vec<RouteService<P>>,
    default: Option<HttpService<P>>,
}

impl<P> Service<ServiceRequest<P>> for ResourceService<P> {
    type Response = ServiceResponse;
    type Error = ();
    type Future = Either<
        Box<Future<Item = ServiceResponse, Error = ()>>,
        Either<
            Box<Future<Item = Self::Response, Error = Self::Error>>,
            FutureResult<Self::Response, Self::Error>,
        >,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, mut req: ServiceRequest<P>) -> Self::Future {
        for route in self.routes.iter_mut() {
            if route.check(&mut req) {
                return Either::A(route.call(req));
            }
        }
        if let Some(ref mut default) = self.default {
            Either::B(Either::A(default.call(req)))
        } else {
            let req = req.into_request();
            Either::B(Either::B(ok(ServiceResponse::new(
                req,
                Response::NotFound().finish(),
            ))))
        }
    }
}

#[doc(hidden)]
pub struct ResourceEndpoint<P> {
    factory: Rc<RefCell<Option<ResourceFactory<P>>>>,
}

impl<P> ResourceEndpoint<P> {
    fn new(factory: Rc<RefCell<Option<ResourceFactory<P>>>>) -> Self {
        ResourceEndpoint { factory }
    }
}

impl<P: 'static> NewService<ServiceRequest<P>> for ResourceEndpoint<P> {
    type Response = ServiceResponse;
    type Error = ();
    type InitError = ();
    type Service = ResourceService<P>;
    type Future = CreateResourceService<P>;

    fn new_service(&self, _: &()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(&())
    }
}

use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use actix_http::{Error, Extensions, Response};
use actix_router::IntoPattern;
use actix_service::boxed::{self, BoxService, BoxServiceFactory};
use actix_service::{
    apply, apply_fn_factory, IntoServiceFactory, Service, ServiceFactory, Transform,
};
use futures_util::future::{ok, Either, LocalBoxFuture, Ready};

use crate::data::Data;
use crate::dev::{insert_slash, AppService, HttpServiceFactory, ResourceDef};
use crate::extract::FromRequest;
use crate::guard::Guard;
use crate::handler::Factory;
use crate::responder::Responder;
use crate::route::{CreateRouteService, Route, RouteService};
use crate::service::{ServiceRequest, ServiceResponse};

type HttpService = BoxService<ServiceRequest, ServiceResponse, Error>;
type HttpNewService = BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>;

/// *Resource* is an entry in resources table which corresponds to requested URL.
///
/// Resource in turn has at least one route.
/// Route consists of an handlers objects and list of guards
/// (objects that implement `Guard` trait).
/// Resources and routes uses builder-like pattern for configuration.
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
/// ```
///
/// If no matching route could be found, *405* response code get returned.
/// Default behavior could be overridden with `default_resource()` method.
pub struct Resource<T = ResourceEndpoint> {
    endpoint: T,
    rdef: Vec<String>,
    name: Option<String>,
    routes: Vec<Route>,
    data: Option<Extensions>,
    guards: Vec<Box<dyn Guard>>,
    default: Rc<RefCell<Option<Rc<HttpNewService>>>>,
    factory_ref: Rc<RefCell<Option<ResourceFactory>>>,
}

impl Resource {
    pub fn new<T: IntoPattern>(path: T) -> Resource {
        let fref = Rc::new(RefCell::new(None));

        Resource {
            routes: Vec::new(),
            rdef: path.patterns(),
            name: None,
            endpoint: ResourceEndpoint::new(fref.clone()),
            factory_ref: fref,
            guards: Vec::new(),
            data: None,
            default: Rc::new(RefCell::new(None)),
        }
    }
}

impl<T> Resource<T>
where
    T: ServiceFactory<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse,
        Error = Error,
        InitError = (),
    >,
{
    /// Set resource name.
    ///
    /// Name is used for url generation.
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    /// Add match guard to a resource.
    ///
    /// ```rust
    /// use actix_web::{web, guard, App, HttpResponse};
    ///
    /// async fn index(data: web::Path<(String, String)>) -> &'static str {
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

    pub(crate) fn add_guards(mut self, guards: Vec<Box<dyn Guard>>) -> Self {
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
    /// Multiple routes could be added to a resource. Resource object uses
    /// match guards for route selection.
    ///
    /// ```rust
    /// use actix_web::{web, guard, App};
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/container/")
    ///              .route(web::get().to(get_handler))
    ///              .route(web::post().to(post_handler))
    ///              .route(web::delete().to(delete_handler))
    ///     );
    /// }
    /// # async fn get_handler() -> impl actix_web::Responder { actix_web::HttpResponse::Ok() }
    /// # async fn post_handler() -> impl actix_web::Responder { actix_web::HttpResponse::Ok() }
    /// # async fn delete_handler() -> impl actix_web::Responder { actix_web::HttpResponse::Ok() }
    /// ```
    pub fn route(mut self, route: Route) -> Self {
        self.routes.push(route);
        self
    }

    /// Provide resource specific data. This method allows to add extractor
    /// configuration or specific state available via `Data<T>` extractor.
    /// Provided data is available for all routes registered for the current resource.
    /// Resource data overrides data registered by `App::data()` method.
    ///
    /// ```rust
    /// use actix_web::{web, App, FromRequest};
    ///
    /// /// extract text data from request
    /// async fn index(body: String) -> String {
    ///     format!("Body {}!", body)
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/index.html")
    ///           // limit size of the payload
    ///           .data(String::configure(|cfg| {
    ///                cfg.limit(4096)
    ///           }))
    ///           .route(
    ///               web::get()
    ///                  // register handler
    ///                  .to(index)
    ///           ));
    /// }
    /// ```
    pub fn data<U: 'static>(self, data: U) -> Self {
        self.app_data(Data::new(data))
    }

    /// Add resource data.
    ///
    /// Data of different types from parent contexts will still be accessible.
    pub fn app_data<U: 'static>(mut self, data: U) -> Self {
        if self.data.is_none() {
            self.data = Some(Extensions::new());
        }
        self.data.as_mut().unwrap().insert(data);
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
    pub fn to<F, I, R, U>(mut self, handler: F) -> Self
    where
        F: Factory<I, R, U>,
        I: FromRequest + 'static,
        R: Future<Output = U> + 'static,
        U: Responder + 'static,
    {
        self.routes.push(Route::new().to(handler));
        self
    }

    /// Register a resource middleware.
    ///
    /// This is similar to `App's` middlewares, but middleware get invoked on resource level.
    /// Resource level middlewares are not allowed to change response
    /// type (i.e modify response's body).
    ///
    /// **Note**: middlewares get called in opposite order of middlewares registration.
    pub fn wrap<M>(
        self,
        mw: M,
    ) -> Resource<
        impl ServiceFactory<
            Config = (),
            Request = ServiceRequest,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            Request = ServiceRequest,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
    {
        Resource {
            endpoint: apply(mw, self.endpoint),
            rdef: self.rdef,
            name: self.name,
            guards: self.guards,
            routes: self.routes,
            default: self.default,
            data: self.data,
            factory_ref: self.factory_ref,
        }
    }

    /// Register a resource middleware function.
    ///
    /// This function accepts instance of `ServiceRequest` type and
    /// mutable reference to the next middleware in chain.
    ///
    /// This is similar to `App's` middlewares, but middleware get invoked on resource level.
    /// Resource level middlewares are not allowed to change response
    /// type (i.e modify response's body).
    ///
    /// ```rust
    /// use actix_service::Service;
    /// use actix_web::{web, App};
    /// use actix_web::http::{header::CONTENT_TYPE, HeaderValue};
    ///
    /// async fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new().service(
    ///         web::resource("/index.html")
    ///             .wrap_fn(|req, srv| {
    ///                 let fut = srv.call(req);
    ///                 async {
    ///                     let mut res = fut.await?;
    ///                     res.headers_mut().insert(
    ///                        CONTENT_TYPE, HeaderValue::from_static("text/plain"),
    ///                     );
    ///                     Ok(res)
    ///                 }
    ///             })
    ///             .route(web::get().to(index)));
    /// }
    /// ```
    pub fn wrap_fn<F, R>(
        self,
        mw: F,
    ) -> Resource<
        impl ServiceFactory<
            Config = (),
            Request = ServiceRequest,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
    >
    where
        F: FnMut(ServiceRequest, &mut T::Service) -> R + Clone,
        R: Future<Output = Result<ServiceResponse, Error>>,
    {
        Resource {
            endpoint: apply_fn_factory(self.endpoint, mw),
            rdef: self.rdef,
            name: self.name,
            guards: self.guards,
            routes: self.routes,
            default: self.default,
            data: self.data,
            factory_ref: self.factory_ref,
        }
    }

    /// Default service to be used if no matching route could be found.
    /// By default *405* response get returned. Resource does not use
    /// default handler from `App` or `Scope`.
    pub fn default_service<F, U>(mut self, f: F) -> Self
    where
        F: IntoServiceFactory<U>,
        U: ServiceFactory<
                Config = (),
                Request = ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
            > + 'static,
        U::InitError: fmt::Debug,
    {
        // create and configure default resource
        self.default = Rc::new(RefCell::new(Some(Rc::new(boxed::factory(
            f.into_factory().map_init_err(|e| {
                log::error!("Can not construct default service: {:?}", e)
            }),
        )))));

        self
    }
}

impl<T> HttpServiceFactory for Resource<T>
where
    T: ServiceFactory<
            Config = (),
            Request = ServiceRequest,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        > + 'static,
{
    fn register(mut self, config: &mut AppService) {
        let guards = if self.guards.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.guards))
        };
        let mut rdef = if config.is_root() || !self.rdef.is_empty() {
            ResourceDef::new(insert_slash(self.rdef.clone()))
        } else {
            ResourceDef::new(self.rdef.clone())
        };
        if let Some(ref name) = self.name {
            *rdef.name_mut() = name.clone();
        }
        // custom app data storage
        if let Some(ref mut ext) = self.data {
            config.set_service_data(ext);
        }

        config.register_service(rdef, guards, self, None)
    }
}

impl<T> IntoServiceFactory<T> for Resource<T>
where
    T: ServiceFactory<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse,
        Error = Error,
        InitError = (),
    >,
{
    fn into_factory(self) -> T {
        *self.factory_ref.borrow_mut() = Some(ResourceFactory {
            routes: self.routes,
            data: self.data.map(Rc::new),
            default: self.default,
        });

        self.endpoint
    }
}

pub struct ResourceFactory {
    routes: Vec<Route>,
    data: Option<Rc<Extensions>>,
    default: Rc<RefCell<Option<Rc<HttpNewService>>>>,
}

impl ServiceFactory for ResourceFactory {
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = ResourceService;
    type Future = CreateResourceService;

    fn new_service(&self, _: ()) -> Self::Future {
        let default_fut = if let Some(ref default) = *self.default.borrow() {
            Some(default.new_service(()))
        } else {
            None
        };

        CreateResourceService {
            fut: self
                .routes
                .iter()
                .map(|route| CreateRouteServiceItem::Future(route.new_service(())))
                .collect(),
            data: self.data.clone(),
            default: None,
            default_fut,
        }
    }
}

enum CreateRouteServiceItem {
    Future(CreateRouteService),
    Service(RouteService),
}

pub struct CreateResourceService {
    fut: Vec<CreateRouteServiceItem>,
    data: Option<Rc<Extensions>>,
    default: Option<HttpService>,
    default_fut: Option<LocalBoxFuture<'static, Result<HttpService, ()>>>,
}

impl Future for CreateResourceService {
    type Output = Result<ResourceService, ()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut done = true;

        if let Some(ref mut fut) = self.default_fut {
            match Pin::new(fut).poll(cx)? {
                Poll::Ready(default) => self.default = Some(default),
                Poll::Pending => done = false,
            }
        }

        // poll http services
        for item in &mut self.fut {
            match item {
                CreateRouteServiceItem::Future(ref mut fut) => match Pin::new(fut)
                    .poll(cx)?
                {
                    Poll::Ready(route) => *item = CreateRouteServiceItem::Service(route),
                    Poll::Pending => {
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
            Poll::Ready(Ok(ResourceService {
                routes,
                data: self.data.clone(),
                default: self.default.take(),
            }))
        } else {
            Poll::Pending
        }
    }
}

pub struct ResourceService {
    routes: Vec<RouteService>,
    data: Option<Rc<Extensions>>,
    default: Option<HttpService>,
}

impl Service for ResourceService {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Either<
        Ready<Result<ServiceResponse, Error>>,
        LocalBoxFuture<'static, Result<ServiceResponse, Error>>,
    >;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: ServiceRequest) -> Self::Future {
        for route in self.routes.iter_mut() {
            if route.check(&mut req) {
                if let Some(ref data) = self.data {
                    req.add_data_container(data.clone());
                }
                return Either::Right(route.call(req));
            }
        }
        if let Some(ref mut default) = self.default {
            if let Some(ref data) = self.data {
                req.add_data_container(data.clone());
            }
            Either::Right(default.call(req))
        } else {
            let req = req.into_parts().0;
            Either::Left(ok(ServiceResponse::new(
                req,
                Response::MethodNotAllowed().finish(),
            )))
        }
    }
}

#[doc(hidden)]
pub struct ResourceEndpoint {
    factory: Rc<RefCell<Option<ResourceFactory>>>,
}

impl ResourceEndpoint {
    fn new(factory: Rc<RefCell<Option<ResourceFactory>>>) -> Self {
        ResourceEndpoint { factory }
    }
}

impl ServiceFactory for ResourceEndpoint {
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Service = ResourceService;
    type Future = CreateResourceService;

    fn new_service(&self, _: ()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actix_rt::time::delay_for;
    use actix_service::Service;
    use futures_util::future::ok;

    use crate::http::{header, HeaderValue, Method, StatusCode};
    use crate::middleware::DefaultHeaders;
    use crate::service::ServiceRequest;
    use crate::test::{call_service, init_service, TestRequest};
    use crate::{guard, web, App, Error, HttpResponse};

    #[actix_rt::test]
    async fn test_middleware() {
        let mut srv =
            init_service(
                App::new().service(
                    web::resource("/test")
                        .name("test")
                        .wrap(DefaultHeaders::new().header(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("0001"),
                        ))
                        .route(web::get().to(HttpResponse::Ok)),
                ),
            )
            .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[actix_rt::test]
    async fn test_middleware_fn() {
        let mut srv = init_service(
            App::new().service(
                web::resource("/test")
                    .wrap_fn(|req, srv| {
                        let fut = srv.call(req);
                        async {
                            fut.await.map(|mut res| {
                                res.headers_mut().insert(
                                    header::CONTENT_TYPE,
                                    HeaderValue::from_static("0001"),
                                );
                                res
                            })
                        }
                    })
                    .route(web::get().to(HttpResponse::Ok)),
            ),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[actix_rt::test]
    async fn test_to() {
        let mut srv =
            init_service(App::new().service(web::resource("/test").to(|| async {
                delay_for(Duration::from_millis(100)).await;
                Ok::<_, Error>(HttpResponse::Ok())
            })))
            .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_pattern() {
        let mut srv = init_service(
            App::new().service(
                web::resource(["/test", "/test2"])
                    .to(|| async { Ok::<_, Error>(HttpResponse::Ok()) }),
            ),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let req = TestRequest::with_uri("/test2").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_default_resource() {
        let mut srv = init_service(
            App::new()
                .service(web::resource("/test").route(web::get().to(HttpResponse::Ok)))
                .default_service(|r: ServiceRequest| {
                    ok(r.into_response(HttpResponse::BadRequest()))
                }),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let mut srv = init_service(
            App::new().service(
                web::resource("/test")
                    .route(web::get().to(HttpResponse::Ok))
                    .default_service(|r: ServiceRequest| {
                        ok(r.into_response(HttpResponse::BadRequest()))
                    }),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_resource_guards() {
        let mut srv = init_service(
            App::new()
                .service(
                    web::resource("/test/{p}")
                        .guard(guard::Get())
                        .to(HttpResponse::Ok),
                )
                .service(
                    web::resource("/test/{p}")
                        .guard(guard::Put())
                        .to(HttpResponse::Created),
                )
                .service(
                    web::resource("/test/{p}")
                        .guard(guard::Delete())
                        .to(HttpResponse::NoContent),
                ),
        )
        .await;

        let req = TestRequest::with_uri("/test/it")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/it")
            .method(Method::PUT)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test/it")
            .method(Method::DELETE)
            .to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[actix_rt::test]
    async fn test_data() {
        let mut srv = init_service(
            App::new()
                .data(1.0f64)
                .data(1usize)
                .app_data(web::Data::new('-'))
                .service(
                    web::resource("/test")
                        .data(10usize)
                        .app_data(web::Data::new('*'))
                        .guard(guard::Get())
                        .to(
                            |data1: web::Data<usize>,
                             data2: web::Data<char>,
                             data3: web::Data<f64>| {
                                assert_eq!(**data1, 10);
                                assert_eq!(**data2, '*');
                                let error = std::f64::EPSILON;
                                assert!((**data3 - 1.0).abs() < error);
                                HttpResponse::Ok()
                            },
                        ),
                ),
        )
        .await;

        let req = TestRequest::get().uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_data_default_service() {
        let mut srv = init_service(
            App::new().data(1usize).service(
                web::resource("/test")
                    .data(10usize)
                    .default_service(web::to(|data: web::Data<usize>| {
                        assert_eq!(**data, 10);
                        HttpResponse::Ok()
                    })),
            ),
        )
        .await;

        let req = TestRequest::get().uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

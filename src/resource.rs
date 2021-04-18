use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::rc::Rc;

use actix_http::{Error, Extensions};
use actix_router::IntoPattern;
use actix_service::boxed::{self, BoxService, BoxServiceFactory};
use actix_service::{
    apply, apply_fn_factory, fn_service, IntoServiceFactory, Service, ServiceFactory,
    ServiceFactoryExt, Transform,
};
use futures_core::future::LocalBoxFuture;
use futures_util::future::join_all;

use crate::dev::{insert_slash, AppService, HttpServiceFactory, ResourceDef};
use crate::extract::FromRequest;
use crate::guard::Guard;
use crate::handler::Handler;
use crate::responder::Responder;
use crate::route::{Route, RouteService};
use crate::service::{ServiceRequest, ServiceResponse};
use crate::{data::Data, HttpResponse};

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
/// ```
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
    app_data: Option<Extensions>,
    guards: Vec<Box<dyn Guard>>,
    default: HttpNewService,
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
            app_data: None,
            default: boxed::factory(fn_service(|req: ServiceRequest| async {
                Ok(req.into_response(HttpResponse::MethodNotAllowed()))
            })),
        }
    }
}

impl<T> Resource<T>
where
    T: ServiceFactory<
        ServiceRequest,
        Config = (),
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
    /// ```
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
    /// ```
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
    /// ```
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
    /// ```
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
        self.app_data
            .get_or_insert_with(Extensions::new)
            .insert(data);

        self
    }

    /// Register a new route and add handler. This route matches all requests.
    ///
    /// ```
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
    /// ```
    /// # extern crate actix_web;
    /// # use actix_web::*;
    /// # fn index(req: HttpRequest) -> HttpResponse { unimplemented!() }
    /// App::new().service(web::resource("/").route(web::route().to(index)));
    /// ```
    pub fn to<F, I, R>(mut self, handler: F) -> Self
    where
        F: Handler<I, R>,
        I: FromRequest + 'static,
        R: Future + 'static,
        R::Output: Responder + 'static,
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
            ServiceRequest,
            Config = (),
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            ServiceRequest,
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
            app_data: self.app_data,
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
    /// ```
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
            ServiceRequest,
            Config = (),
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        >,
    >
    where
        F: Fn(ServiceRequest, &T::Service) -> R + Clone,
        R: Future<Output = Result<ServiceResponse, Error>>,
    {
        Resource {
            endpoint: apply_fn_factory(self.endpoint, mw),
            rdef: self.rdef,
            name: self.name,
            guards: self.guards,
            routes: self.routes,
            default: self.default,
            app_data: self.app_data,
            factory_ref: self.factory_ref,
        }
    }

    /// Default service to be used if no matching route could be found.
    /// By default *405* response get returned. Resource does not use
    /// default handler from `App` or `Scope`.
    pub fn default_service<F, U>(mut self, f: F) -> Self
    where
        F: IntoServiceFactory<U, ServiceRequest>,
        U: ServiceFactory<
                ServiceRequest,
                Config = (),
                Response = ServiceResponse,
                Error = Error,
            > + 'static,
        U::InitError: fmt::Debug,
    {
        // create and configure default resource
        self.default = boxed::factory(
            f.into_factory()
                .map_init_err(|e| log::error!("Can not construct default service: {:?}", e)),
        );

        self
    }
}

impl<T> HttpServiceFactory for Resource<T>
where
    T: ServiceFactory<
            ServiceRequest,
            Config = (),
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

        config.register_service(rdef, guards, self, None)
    }
}

impl<T> IntoServiceFactory<T, ServiceRequest> for Resource<T>
where
    T: ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse,
        Error = Error,
        InitError = (),
    >,
{
    fn into_factory(self) -> T {
        *self.factory_ref.borrow_mut() = Some(ResourceFactory {
            routes: self.routes,
            app_data: self.app_data.map(Rc::new),
            default: self.default,
        });

        self.endpoint
    }
}

pub struct ResourceFactory {
    routes: Vec<Route>,
    app_data: Option<Rc<Extensions>>,
    default: HttpNewService,
}

impl ServiceFactory<ServiceRequest> for ResourceFactory {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = ResourceService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        // construct default service factory future.
        let default_fut = self.default.new_service(());

        // construct route service factory futures
        let factory_fut = join_all(self.routes.iter().map(|route| route.new_service(())));

        let app_data = self.app_data.clone();

        Box::pin(async move {
            let default = default_fut.await?;
            let routes = factory_fut
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            Ok(ResourceService {
                routes,
                app_data,
                default,
            })
        })
    }
}

pub struct ResourceService {
    routes: Vec<RouteService>,
    app_data: Option<Rc<Extensions>>,
    default: HttpService,
}

impl Service<ServiceRequest> for ResourceService {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        for route in self.routes.iter() {
            if route.check(&mut req) {
                if let Some(ref app_data) = self.app_data {
                    req.add_data_container(app_data.clone());
                }

                return route.call(req);
            }
        }

        if let Some(ref app_data) = self.app_data {
            req.add_data_container(app_data.clone());
        }

        self.default.call(req)
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

impl ServiceFactory<ServiceRequest> for ResourceEndpoint {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = ResourceService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        self.factory.borrow().as_ref().unwrap().new_service(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use actix_rt::time::sleep;
    use actix_service::Service;
    use actix_utils::future::ok;

    use crate::http::{header, HeaderValue, Method, StatusCode};
    use crate::middleware::DefaultHeaders;
    use crate::service::ServiceRequest;
    use crate::test::{call_service, init_service, TestRequest};
    use crate::{guard, web, App, Error, HttpResponse};

    #[actix_rt::test]
    async fn test_middleware() {
        let srv = init_service(
            App::new().service(
                web::resource("/test")
                    .name("test")
                    .wrap(
                        DefaultHeaders::new()
                            .header(header::CONTENT_TYPE, HeaderValue::from_static("0001")),
                    )
                    .route(web::get().to(HttpResponse::Ok)),
            ),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[actix_rt::test]
    async fn test_middleware_fn() {
        let srv = init_service(
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
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[actix_rt::test]
    async fn test_to() {
        let srv = init_service(App::new().service(web::resource("/test").to(|| async {
            sleep(Duration::from_millis(100)).await;
            Ok::<_, Error>(HttpResponse::Ok())
        })))
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_pattern() {
        let srv = init_service(
            App::new().service(
                web::resource(["/test", "/test2"])
                    .to(|| async { Ok::<_, Error>(HttpResponse::Ok()) }),
            ),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let req = TestRequest::with_uri("/test2").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_default_resource() {
        let srv = init_service(
            App::new()
                .service(web::resource("/test").route(web::get().to(HttpResponse::Ok)))
                .default_service(|r: ServiceRequest| {
                    ok(r.into_response(HttpResponse::BadRequest()))
                }),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let srv = init_service(
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
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test")
            .method(Method::POST)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_resource_guards() {
        let srv = init_service(
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
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test/it")
            .method(Method::PUT)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/test/it")
            .method(Method::DELETE)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[actix_rt::test]
    async fn test_data() {
        let srv = init_service(
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
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_data_default_service() {
        let srv = init_service(
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
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

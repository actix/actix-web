use std::{cell::RefCell, fmt, future::Future, rc::Rc};

use actix_http::Extensions;
use actix_router::{IntoPatterns, Patterns};
use actix_service::{
    apply, apply_fn_factory, boxed, fn_service, IntoServiceFactory, Service, ServiceFactory,
    ServiceFactoryExt, Transform,
};
use futures_core::future::LocalBoxFuture;
use futures_util::future::join_all;

use crate::{
    body::MessageBody,
    data::Data,
    dev::{ensure_leading_slash, AppService, ResourceDef},
    guard::{self, Guard},
    handler::Handler,
    http::header,
    route::{Route, RouteService},
    service::{
        BoxedHttpService, BoxedHttpServiceFactory, HttpServiceFactory, ServiceRequest,
        ServiceResponse,
    },
    web, Error, FromRequest, HttpResponse, Responder,
};

/// A collection of [`Route`]s that respond to the same path pattern.
///
/// Resource in turn has at least one route. Route consists of an handlers objects and list of
/// guards (objects that implement `Guard` trait). Resources and routes uses builder-like pattern
/// for configuration. During request handling, resource object iterate through all routes and check
/// guards for specific route, if request matches all guards, route considered matched and route
/// handler get called.
///
/// # Examples
/// ```
/// use actix_web::{web, App, HttpResponse};
///
/// let app = App::new().service(
///     web::resource("/")
///         .get(|| HttpResponse::Ok())
///         .post(|| async { "Hello World!" })
/// );
/// ```
///
/// If no matching route is found, an empty 405 response is returned which includes an
/// [appropriate Allow header][RFC 9110 §15.5.6]. This default behavior can be overridden using
/// [`default_service()`](Self::default_service).
///
/// [RFC 9110 §15.5.6]: https://www.rfc-editor.org/rfc/rfc9110.html#section-15.5.6
pub struct Resource<T = ResourceEndpoint> {
    endpoint: T,
    rdef: Patterns,
    name: Option<String>,
    routes: Vec<Route>,
    app_data: Option<Extensions>,
    guards: Vec<Box<dyn Guard>>,
    default: BoxedHttpServiceFactory,
    factory_ref: Rc<RefCell<Option<ResourceFactory>>>,
}

impl Resource {
    /// Constructs new resource that matches a `path` pattern.
    pub fn new<T: IntoPatterns>(path: T) -> Resource {
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
                use crate::HttpMessage as _;

                let allowed = req.extensions().get::<guard::RegisteredMethods>().cloned();

                if let Some(methods) = allowed {
                    Ok(req.into_response(
                        HttpResponse::MethodNotAllowed()
                            .insert_header(header::Allow(methods.0))
                            .finish(),
                    ))
                } else {
                    Ok(req.into_response(HttpResponse::MethodNotAllowed()))
                }
            })),
        }
    }
}

impl<T> Resource<T>
where
    T: ServiceFactory<ServiceRequest, Config = (), Error = Error, InitError = ()>,
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
    /// let app = App::new()
    ///     .service(
    ///         web::resource("/app")
    ///             .guard(guard::Header("content-type", "text/plain"))
    ///             .route(web::get().to(index))
    ///     )
    ///     .service(
    ///         web::resource("/app")
    ///             .guard(guard::Header("content-type", "text/json"))
    ///             .route(web::get().to(|| HttpResponse::MethodNotAllowed()))
    ///     );
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
    /// let app = App::new().service(
    ///     web::resource("/").route(
    ///         web::route()
    ///             .guard(guard::Any(guard::Get()).or(guard::Put()))
    ///             .guard(guard::Header("Content-Type", "text/plain"))
    ///             .to(|| HttpResponse::Ok()))
    /// );
    /// ```
    ///
    /// Multiple routes could be added to a resource. Resource object uses
    /// match guards for route selection.
    ///
    /// ```
    /// use actix_web::{web, guard, App};
    ///
    /// let app = App::new().service(
    ///     web::resource("/container/")
    ///          .route(web::get().to(get_handler))
    ///          .route(web::post().to(post_handler))
    ///          .route(web::delete().to(delete_handler))
    /// );
    ///
    /// # async fn get_handler() -> impl actix_web::Responder { actix_web::HttpResponse::Ok() }
    /// # async fn post_handler() -> impl actix_web::Responder { actix_web::HttpResponse::Ok() }
    /// # async fn delete_handler() -> impl actix_web::Responder { actix_web::HttpResponse::Ok() }
    /// ```
    pub fn route(mut self, route: Route) -> Self {
        self.routes.push(route);
        self
    }

    /// Add resource data.
    ///
    /// Data of different types from parent contexts will still be accessible. Any `Data<T>` types
    /// set here can be extracted in handlers using the `Data<T>` extractor.
    ///
    /// # Examples
    /// ```
    /// use std::cell::Cell;
    /// use actix_web::{web, App, HttpRequest, HttpResponse, Responder};
    ///
    /// struct MyData {
    ///     count: std::cell::Cell<usize>,
    /// }
    ///
    /// async fn handler(req: HttpRequest, counter: web::Data<MyData>) -> impl Responder {
    ///     // note this cannot use the Data<T> extractor because it was not added with it
    ///     let incr = *req.app_data::<usize>().unwrap();
    ///     assert_eq!(incr, 3);
    ///
    ///     // update counter using other value from app data
    ///     counter.count.set(counter.count.get() + incr);
    ///
    ///     HttpResponse::Ok().body(counter.count.get().to_string())
    /// }
    ///
    /// let app = App::new().service(
    ///     web::resource("/")
    ///         .app_data(3usize)
    ///         .app_data(web::Data::new(MyData { count: Default::default() }))
    ///         .route(web::get().to(handler))
    /// );
    /// ```
    #[doc(alias = "manage")]
    pub fn app_data<U: 'static>(mut self, data: U) -> Self {
        self.app_data
            .get_or_insert_with(Extensions::new)
            .insert(data);

        self
    }

    /// Add resource data after wrapping in `Data<T>`.
    ///
    /// Deprecated in favor of [`app_data`](Self::app_data).
    #[deprecated(since = "4.0.0", note = "Use `.app_data(Data::new(val))` instead.")]
    pub fn data<U: 'static>(self, data: U) -> Self {
        self.app_data(Data::new(data))
    }

    /// Register a new route and add handler. This route matches all requests.
    ///
    /// ```
    /// use actix_web::{App, HttpRequest, HttpResponse, web};
    ///
    /// async fn index(req: HttpRequest) -> HttpResponse {
    ///     todo!()
    /// }
    ///
    /// App::new().service(web::resource("/").to(index));
    /// ```
    ///
    /// This is shortcut for:
    ///
    /// ```
    /// # use actix_web::*;
    /// # async fn index(req: HttpRequest) -> HttpResponse { todo!() }
    /// App::new().service(web::resource("/").route(web::route().to(index)));
    /// ```
    pub fn to<F, Args>(mut self, handler: F) -> Self
    where
        F: Handler<Args>,
        Args: FromRequest + 'static,
        F::Output: Responder + 'static,
    {
        self.routes.push(Route::new().to(handler));
        self
    }

    /// Registers a resource middleware.
    ///
    /// `mw` is a middleware component (type), that can modify the request and response across all
    /// routes managed by this `Resource`.
    ///
    /// See [`App::wrap`](crate::App::wrap) for more details.
    #[doc(alias = "middleware")]
    #[doc(alias = "use")] // nodejs terminology
    pub fn wrap<M, B>(
        self,
        mw: M,
    ) -> Resource<
        impl ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
                T::Service,
                ServiceRequest,
                Response = ServiceResponse<B>,
                Error = Error,
                InitError = (),
            > + 'static,
        B: MessageBody,
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

    /// Registers a resource function middleware.
    ///
    /// `mw` is a closure that runs during inbound and/or outbound processing in the request
    /// life-cycle (request -> response), modifying request/response as necessary, across all
    /// requests handled by the `Resource`.
    ///
    /// See [`App::wrap_fn`](crate::App::wrap_fn) for examples and more details.
    #[doc(alias = "middleware")]
    #[doc(alias = "use")] // nodejs terminology
    pub fn wrap_fn<F, R, B>(
        self,
        mw: F,
    ) -> Resource<
        impl ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        F: Fn(ServiceRequest, &T::Service) -> R + Clone + 'static,
        R: Future<Output = Result<ServiceResponse<B>, Error>>,
        B: MessageBody,
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

    /// Sets the default service to be used if no matching route is found.
    ///
    /// Unlike [`Scope`]s, a `Resource` does _not_ inherit its parent's default service. You can
    /// use a [`Route`] as default service.
    ///
    /// If a custom default service is not registered, an empty `405 Method Not Allowed` response
    /// with an appropriate Allow header will be sent instead.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{App, HttpResponse, web};
    ///
    /// let resource = web::resource("/test")
    ///     .route(web::get().to(HttpResponse::Ok))
    ///     .default_service(web::to(|| {
    ///         HttpResponse::BadRequest()
    ///     }));
    ///
    /// App::new().service(resource);
    /// ```
    ///
    /// [`Scope`]: crate::Scope
    pub fn default_service<F, U>(mut self, f: F) -> Self
    where
        F: IntoServiceFactory<U, ServiceRequest>,
        U: ServiceFactory<ServiceRequest, Config = (), Response = ServiceResponse, Error = Error>
            + 'static,
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

macro_rules! route_shortcut {
    ($method_fn:ident, $method_upper:literal) => {
        #[doc = concat!(" Adds a ", $method_upper, " route.")]
        ///
        /// Use [`route`](Self::route) if you need to add additional guards.
        ///
        /// # Examples
        ///
        /// ```
        /// # use actix_web::web;
        /// web::resource("/")
        #[doc = concat!("    .", stringify!($method_fn), "(|| async { \"Hello World!\" })")]
        /// # ;
        /// ```
        pub fn $method_fn<F, Args>(self, handler: F) -> Self
        where
            F: Handler<Args>,
            Args: FromRequest + 'static,
            F::Output: Responder + 'static,
        {
            self.route(web::$method_fn().to(handler))
        }
    };
}

/// Concise routes for well-known HTTP methods.
impl<T> Resource<T>
where
    T: ServiceFactory<ServiceRequest, Config = (), Error = Error, InitError = ()>,
{
    route_shortcut!(get, "GET");
    route_shortcut!(post, "POST");
    route_shortcut!(put, "PUT");
    route_shortcut!(patch, "PATCH");
    route_shortcut!(delete, "DELETE");
    route_shortcut!(head, "HEAD");
    route_shortcut!(trace, "TRACE");
}

impl<T, B> HttpServiceFactory for Resource<T>
where
    T: ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        > + 'static,
    B: MessageBody + 'static,
{
    fn register(mut self, config: &mut AppService) {
        let guards = if self.guards.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.guards))
        };

        let mut rdef = if config.is_root() || !self.rdef.is_empty() {
            ResourceDef::new(ensure_leading_slash(self.rdef.clone()))
        } else {
            ResourceDef::new(self.rdef.clone())
        };

        if let Some(ref name) = self.name {
            rdef.set_name(name);
        }

        *self.factory_ref.borrow_mut() = Some(ResourceFactory {
            routes: self.routes,
            default: self.default,
        });

        let resource_data = self.app_data.map(Rc::new);

        // wraps endpoint service (including middleware) call and injects app data for this scope
        let endpoint = apply_fn_factory(self.endpoint, move |mut req: ServiceRequest, srv| {
            if let Some(ref data) = resource_data {
                req.add_data_container(Rc::clone(data));
            }

            let fut = srv.call(req);

            async { Ok(fut.await?.map_into_boxed_body()) }
        });

        config.register_service(rdef, guards, endpoint, None)
    }
}

pub struct ResourceFactory {
    routes: Vec<Route>,
    default: BoxedHttpServiceFactory,
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

        Box::pin(async move {
            let default = default_fut.await?;
            let routes = factory_fut
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            Ok(ResourceService { routes, default })
        })
    }
}

pub struct ResourceService {
    routes: Vec<RouteService>,
    default: BoxedHttpService,
}

impl Service<ServiceRequest> for ResourceService {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        for route in &self.routes {
            if route.check(&mut req) {
                return route.call(req);
            }
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
    use actix_utils::future::ok;

    use super::*;
    use crate::{
        http::{header::HeaderValue, Method, StatusCode},
        middleware::DefaultHeaders,
        test::{call_service, init_service, TestRequest},
        App, HttpMessage,
    };

    #[test]
    fn can_be_returned_from_fn() {
        fn my_resource_1() -> Resource {
            web::resource("/test1").route(web::get().to(|| async { "hello" }))
        }

        fn my_resource_2() -> Resource<
            impl ServiceFactory<
                ServiceRequest,
                Config = (),
                Response = ServiceResponse<impl MessageBody>,
                Error = Error,
                InitError = (),
            >,
        > {
            web::resource("/test2")
                .wrap_fn(|req, srv| {
                    let fut = srv.call(req);
                    async { Ok(fut.await?.map_into_right_body::<()>()) }
                })
                .route(web::get().to(|| async { "hello" }))
        }

        fn my_resource_3() -> impl HttpServiceFactory {
            web::resource("/test3").route(web::get().to(|| async { "hello" }))
        }

        App::new()
            .service(my_resource_1())
            .service(my_resource_2())
            .service(my_resource_3());
    }

    #[actix_rt::test]
    async fn test_middleware() {
        let srv = init_service(
            App::new().service(
                web::resource("/test")
                    .name("test")
                    .wrap(
                        DefaultHeaders::new()
                            .add((header::CONTENT_TYPE, HeaderValue::from_static("0001"))),
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
                                res.headers_mut()
                                    .insert(header::CONTENT_TYPE, HeaderValue::from_static("0001"));
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
        let srv = init_service(App::new().service(
            web::resource(["/test", "/test2"]).to(|| async { Ok::<_, Error>(HttpResponse::Ok()) }),
        ))
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
                .service(
                    web::resource("/test")
                        .route(web::get().to(HttpResponse::Ok))
                        .route(web::delete().to(HttpResponse::Ok)),
                )
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
        assert_eq!(
            resp.headers().get(header::ALLOW).unwrap().as_bytes(),
            b"GET, DELETE"
        );

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

    // allow deprecated `{App, Resource}::data`
    #[allow(deprecated)]
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
                                let error = f64::EPSILON;
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

    // allow deprecated `{App, Resource}::data`
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_data_default_service() {
        let srv =
            init_service(
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

    #[actix_rt::test]
    async fn test_middleware_app_data() {
        let srv = init_service(
            App::new().service(
                web::resource("test")
                    .app_data(1usize)
                    .wrap_fn(|req, srv| {
                        assert_eq!(req.app_data::<usize>(), Some(&1usize));
                        req.extensions_mut().insert(1usize);
                        srv.call(req)
                    })
                    .route(web::get().to(HttpResponse::Ok))
                    .default_service(|req: ServiceRequest| async move {
                        let (req, _) = req.into_parts();

                        assert_eq!(req.extensions().get::<usize>(), Some(&1));

                        Ok(ServiceResponse::new(
                            req,
                            HttpResponse::BadRequest().finish(),
                        ))
                    }),
            ),
        )
        .await;

        let req = TestRequest::get().uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::post().uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn test_middleware_body_type() {
        let srv = init_service(
            App::new().service(
                web::resource("/test")
                    .wrap_fn(|req, srv| {
                        let fut = srv.call(req);
                        async { Ok(fut.await?.map_into_right_body::<()>()) }
                    })
                    .route(web::get().to(|| async { "hello" })),
            ),
        )
        .await;

        // test if `try_into_bytes()` is preserved across scope layer
        use actix_http::body::MessageBody as _;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        let body = resp.into_body();
        assert_eq!(body.try_into_bytes().unwrap(), b"hello".as_ref());
    }
}

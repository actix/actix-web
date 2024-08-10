use std::{cell::RefCell, fmt, future::Future, mem, rc::Rc};

use actix_http::{body::MessageBody, Extensions};
use actix_router::{ResourceDef, Router};
use actix_service::{
    apply, apply_fn_factory, boxed, IntoServiceFactory, Service, ServiceFactory, ServiceFactoryExt,
    Transform,
};
use futures_core::future::LocalBoxFuture;
use futures_util::future::join_all;

use crate::{
    config::ServiceConfig,
    data::Data,
    dev::AppService,
    guard::Guard,
    rmap::ResourceMap,
    service::{
        AppServiceFactory, BoxedHttpService, BoxedHttpServiceFactory, HttpServiceFactory,
        ServiceFactoryWrapper, ServiceRequest, ServiceResponse,
    },
    Error, Resource, Route,
};

type Guards = Vec<Box<dyn Guard>>;

/// A collection of [`Route`]s, [`Resource`]s, or other services that share a common path prefix.
///
/// The `Scope`'s path can contain [dynamic segments]. The dynamic segments can be extracted from
/// requests using the [`Path`](crate::web::Path) extractor or
/// with [`HttpRequest::match_info()`](crate::HttpRequest::match_info).
///
/// # Avoid Trailing Slashes
/// Avoid using trailing slashes in the scope prefix (e.g., `web::scope("/scope/")`). It will almost
/// certainly not have the expected behavior. See the [documentation on resource definitions][pat]
/// to understand why this is the case and how to correctly construct scope/prefix definitions.
///
/// # Examples
/// ```
/// use actix_web::{web, App, HttpResponse};
///
/// let app = App::new().service(
///     web::scope("/{project_id}")
///         .service(web::resource("/path1").to(|| async { "OK" }))
///         .service(web::resource("/path2").route(web::get().to(|| HttpResponse::Ok())))
///         .service(web::resource("/path3").route(web::head().to(HttpResponse::MethodNotAllowed)))
/// );
/// ```
///
/// In the above example three routes get registered:
/// - /{project_id}/path1 - responds to all HTTP methods
/// - /{project_id}/path2 - responds to `GET` requests
/// - /{project_id}/path3 - responds to `HEAD` requests
///
/// [pat]: crate::dev::ResourceDef#prefix-resources
/// [dynamic segments]: crate::dev::ResourceDef#dynamic-segments
pub struct Scope<T = ScopeEndpoint> {
    endpoint: T,
    rdef: String,
    app_data: Option<Extensions>,
    services: Vec<Box<dyn AppServiceFactory>>,
    guards: Vec<Box<dyn Guard>>,
    default: Option<Rc<BoxedHttpServiceFactory>>,
    external: Vec<ResourceDef>,
    factory_ref: Rc<RefCell<Option<ScopeFactory>>>,
}

impl Scope {
    /// Create a new scope
    pub fn new(path: &str) -> Scope {
        let factory_ref = Rc::new(RefCell::new(None));

        Scope {
            endpoint: ScopeEndpoint::new(Rc::clone(&factory_ref)),
            rdef: path.to_string(),
            app_data: None,
            guards: Vec::new(),
            services: Vec::new(),
            default: None,
            external: Vec::new(),
            factory_ref,
        }
    }
}

impl<T> Scope<T>
where
    T: ServiceFactory<ServiceRequest, Config = (), Error = Error, InitError = ()>,
{
    /// Add match guard to a scope.
    ///
    /// ```
    /// use actix_web::{web, guard, App, HttpRequest, HttpResponse};
    ///
    /// async fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// let app = App::new().service(
    ///     web::scope("/app")
    ///         .guard(guard::Header("content-type", "text/plain"))
    ///         .route("/test1", web::get().to(index))
    ///         .route("/test2", web::post().to(|r: HttpRequest| {
    ///             HttpResponse::MethodNotAllowed()
    ///         }))
    /// );
    /// ```
    pub fn guard<G: Guard + 'static>(mut self, guard: G) -> Self {
        self.guards.push(Box::new(guard));
        self
    }

    /// Add scope data.
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
    ///     web::scope("/app")
    ///         .app_data(3usize)
    ///         .app_data(web::Data::new(MyData { count: Default::default() }))
    ///         .route("/", web::get().to(handler))
    /// );
    /// ```
    #[doc(alias = "manage")]
    pub fn app_data<U: 'static>(mut self, data: U) -> Self {
        self.app_data
            .get_or_insert_with(Extensions::new)
            .insert(data);

        self
    }

    /// Add scope data after wrapping in `Data<T>`.
    ///
    /// Deprecated in favor of [`app_data`](Self::app_data).
    #[deprecated(since = "4.0.0", note = "Use `.app_data(Data::new(val))` instead.")]
    pub fn data<U: 'static>(self, data: U) -> Self {
        self.app_data(Data::new(data))
    }

    /// Run external configuration as part of the scope building process.
    ///
    /// This function is useful for moving parts of configuration to a different module or library.
    /// For example, some of the resource's configuration could be moved to different module.
    ///
    /// ```
    /// use actix_web::{web, middleware, App, HttpResponse};
    ///
    /// // this function could be located in different module
    /// fn config(cfg: &mut web::ServiceConfig) {
    ///     cfg.service(web::resource("/test")
    ///         .route(web::get().to(|| HttpResponse::Ok()))
    ///         .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
    ///     );
    /// }
    ///
    /// let app = App::new()
    ///     .wrap(middleware::Logger::default())
    ///     .service(
    ///         web::scope("/api")
    ///             .configure(config)
    ///     )
    ///     .route("/index.html", web::get().to(|| HttpResponse::Ok()));
    /// ```
    pub fn configure<F>(mut self, cfg_fn: F) -> Self
    where
        F: FnOnce(&mut ServiceConfig),
    {
        let mut cfg = ServiceConfig::new();
        cfg_fn(&mut cfg);

        self.services.extend(cfg.services);
        self.external.extend(cfg.external);

        // TODO: add Extensions::is_empty check and conditionally insert data
        self.app_data
            .get_or_insert_with(Extensions::new)
            .extend(cfg.app_data);

        if let Some(default) = cfg.default {
            self.default = Some(default);
        }

        self
    }

    /// Register HTTP service.
    ///
    /// This is similar to `App's` service registration.
    ///
    /// Actix Web provides several services implementations:
    ///
    /// * *Resource* is an entry in resource table which corresponds to requested URL.
    /// * *Scope* is a set of resources with common root path.
    ///
    /// ```
    /// use actix_web::{web, App, HttpRequest};
    ///
    /// struct AppState;
    ///
    /// async fn index(req: HttpRequest) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// let app = App::new().service(
    ///     web::scope("/app").service(
    ///         web::scope("/v1")
    ///             .service(web::resource("/test1").to(index)))
    /// );
    /// ```
    pub fn service<F>(mut self, factory: F) -> Self
    where
        F: HttpServiceFactory + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `Scope::service()` method.
    /// This method can be called multiple times, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// async fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// let app = App::new().service(
    ///     web::scope("/app")
    ///         .route("/test1", web::get().to(index))
    ///         .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()))
    /// );
    /// ```
    pub fn route(self, path: &str, mut route: Route) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Default service to be used if no matching resource could be found.
    ///
    /// If a default service is not registered, it will fall back to the default service of
    /// the parent [`App`](crate::App) (see [`App::default_service`](crate::App::default_service)).
    pub fn default_service<F, U>(mut self, f: F) -> Self
    where
        F: IntoServiceFactory<U, ServiceRequest>,
        U: ServiceFactory<ServiceRequest, Config = (), Response = ServiceResponse, Error = Error>
            + 'static,
        U::InitError: fmt::Debug,
    {
        // create and configure default resource
        self.default = Some(Rc::new(boxed::factory(f.into_factory().map_init_err(
            |e| log::error!("Can not construct default service: {:?}", e),
        ))));

        self
    }

    /// Registers a scope-wide middleware.
    ///
    /// `mw` is a middleware component (type), that can modify the request and response across all
    /// sub-resources managed by this `Scope`.
    ///
    /// See [`App::wrap`](crate::App::wrap) for more details.
    #[doc(alias = "middleware")]
    #[doc(alias = "use")] // nodejs terminology
    pub fn wrap<M, B>(
        self,
        mw: M,
    ) -> Scope<
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
        Scope {
            endpoint: apply(mw, self.endpoint),
            rdef: self.rdef,
            app_data: self.app_data,
            guards: self.guards,
            services: self.services,
            default: self.default,
            external: self.external,
            factory_ref: self.factory_ref,
        }
    }

    /// Registers a scope-wide function middleware.
    ///
    /// `mw` is a closure that runs during inbound and/or outbound processing in the request
    /// life-cycle (request -> response), modifying request/response as necessary, across all
    /// requests handled by the `Scope`.
    ///
    /// See [`App::wrap_fn`](crate::App::wrap_fn) for examples and more details.
    #[doc(alias = "middleware")]
    #[doc(alias = "use")] // nodejs terminology
    pub fn wrap_fn<F, R, B>(
        self,
        mw: F,
    ) -> Scope<
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
        Scope {
            endpoint: apply_fn_factory(self.endpoint, mw),
            rdef: self.rdef,
            app_data: self.app_data,
            guards: self.guards,
            services: self.services,
            default: self.default,
            external: self.external,
            factory_ref: self.factory_ref,
        }
    }
}

impl<T, B> HttpServiceFactory for Scope<T>
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
        // update default resource if needed
        let default = self.default.unwrap_or_else(|| config.default_service());

        // register nested services
        let mut cfg = config.clone_config();
        self.services
            .into_iter()
            .for_each(|mut srv| srv.register(&mut cfg));

        let mut rmap = ResourceMap::new(ResourceDef::root_prefix(&self.rdef));

        // external resources
        for mut rdef in mem::take(&mut self.external) {
            rmap.add(&mut rdef, None);
        }

        // complete scope pipeline creation
        *self.factory_ref.borrow_mut() = Some(ScopeFactory {
            default,
            services: cfg
                .into_services()
                .1
                .into_iter()
                .map(|(mut rdef, srv, guards, nested)| {
                    rmap.add(&mut rdef, nested);
                    (rdef, srv, RefCell::new(guards))
                })
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .into(),
        });

        // get guards
        let guards = if self.guards.is_empty() {
            None
        } else {
            Some(self.guards)
        };

        let scope_data = self.app_data.map(Rc::new);

        // wraps endpoint service (including middleware) call and injects app data for this scope
        let endpoint = apply_fn_factory(self.endpoint, move |mut req: ServiceRequest, srv| {
            if let Some(ref data) = scope_data {
                req.add_data_container(Rc::clone(data));
            }

            let fut = srv.call(req);

            async { Ok(fut.await?.map_into_boxed_body()) }
        });

        // register final service
        config.register_service(
            ResourceDef::root_prefix(&self.rdef),
            guards,
            endpoint,
            Some(Rc::new(rmap)),
        )
    }
}

pub struct ScopeFactory {
    #[allow(clippy::type_complexity)]
    services: Rc<
        [(
            ResourceDef,
            BoxedHttpServiceFactory,
            RefCell<Option<Guards>>,
        )],
    >,
    default: Rc<BoxedHttpServiceFactory>,
}

impl ServiceFactory<ServiceRequest> for ScopeFactory {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = ScopeService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        // construct default service factory future
        let default_fut = self.default.new_service(());

        // construct all services factory future with it's resource def and guards.
        let factory_fut = join_all(self.services.iter().map(|(path, factory, guards)| {
            let path = path.clone();
            let guards = guards.borrow_mut().take().unwrap_or_default();
            let factory_fut = factory.new_service(());
            async move {
                factory_fut
                    .await
                    .map(move |service| (path, guards, service))
            }
        }));

        Box::pin(async move {
            let default = default_fut.await?;

            // build router from the factory future result.
            let router = factory_fut
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
                .drain(..)
                .fold(Router::build(), |mut router, (path, guards, service)| {
                    router.push(path, service, guards);
                    router
                })
                .finish();

            Ok(ScopeService { router, default })
        })
    }
}

pub struct ScopeService {
    router: Router<BoxedHttpService, Vec<Box<dyn Guard>>>,
    default: BoxedHttpService,
}

impl Service<ServiceRequest> for ScopeService {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let res = self.router.recognize_fn(&mut req, |req, guards| {
            let guard_ctx = req.guard_ctx();
            guards.iter().all(|guard| guard.check(&guard_ctx))
        });

        if let Some((srv, _info)) = res {
            srv.call(req)
        } else {
            self.default.call(req)
        }
    }
}

#[doc(hidden)]
pub struct ScopeEndpoint {
    factory: Rc<RefCell<Option<ScopeFactory>>>,
}

impl ScopeEndpoint {
    fn new(factory: Rc<RefCell<Option<ScopeFactory>>>) -> Self {
        ScopeEndpoint { factory }
    }
}

impl ServiceFactory<ServiceRequest> for ScopeEndpoint {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = ScopeService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        self.factory.borrow_mut().as_mut().unwrap().new_service(())
    }
}

#[cfg(test)]
mod tests {
    use actix_utils::future::ok;
    use bytes::Bytes;

    use super::*;
    use crate::{
        guard,
        http::{
            header::{self, HeaderValue},
            Method, StatusCode,
        },
        middleware::DefaultHeaders,
        test::{assert_body_eq, call_service, init_service, read_body, TestRequest},
        web, App, HttpMessage, HttpRequest, HttpResponse,
    };

    #[test]
    fn can_be_returned_from_fn() {
        fn my_scope_1() -> Scope {
            web::scope("/test")
                .service(web::resource("").route(web::get().to(|| async { "hello" })))
        }

        fn my_scope_2() -> Scope<
            impl ServiceFactory<
                ServiceRequest,
                Config = (),
                Response = ServiceResponse<impl MessageBody>,
                Error = Error,
                InitError = (),
            >,
        > {
            web::scope("/test-compat")
                .wrap_fn(|req, srv| {
                    let fut = srv.call(req);
                    async { Ok(fut.await?.map_into_right_body::<()>()) }
                })
                .service(web::resource("").route(web::get().to(|| async { "hello" })))
        }

        fn my_scope_3() -> impl HttpServiceFactory {
            my_scope_2()
        }

        App::new()
            .service(my_scope_1())
            .service(my_scope_2())
            .service(my_scope_3());
    }

    #[actix_rt::test]
    async fn test_scope() {
        let srv = init_service(
            App::new()
                .service(web::scope("/app").service(web::resource("/path1").to(HttpResponse::Ok))),
        )
        .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_root() {
        let srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::resource("").to(HttpResponse::Ok))
                    .service(web::resource("/").to(HttpResponse::Created)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_scope_root2() {
        let srv = init_service(
            App::new().service(web::scope("/app/").service(web::resource("").to(HttpResponse::Ok))),
        )
        .await;

        let req = TestRequest::with_uri("/app").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_root3() {
        let srv = init_service(
            App::new()
                .service(web::scope("/app/").service(web::resource("/").to(HttpResponse::Ok))),
        )
        .await;

        let req = TestRequest::with_uri("/app").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_scope_route() {
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .route("/path1", web::get().to(HttpResponse::Ok))
                    .route("/path1", web::delete().to(HttpResponse::Ok)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_scope_route_without_leading_slash() {
        let srv = init_service(
            App::new().service(
                web::scope("app").service(
                    web::resource("path1")
                        .route(web::get().to(HttpResponse::Ok))
                        .route(web::delete().to(HttpResponse::Ok)),
                ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::DELETE)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[actix_rt::test]
    async fn test_scope_guard() {
        let srv = init_service(
            App::new().service(
                web::scope("/app")
                    .guard(guard::Get())
                    .service(web::resource("/path1").to(HttpResponse::Ok)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/path1")
            .method(Method::GET)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_variable_segment() {
        let srv = init_service(App::new().service(web::scope("/ab-{project}").service(
            web::resource("/path1").to(|r: HttpRequest| {
                HttpResponse::Ok().body(format!("project: {}", &r.match_info()["project"]))
            }),
        )))
        .await;

        let req = TestRequest::with_uri("/ab-project1/path1").to_request();
        let res = srv.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_body_eq!(res, b"project: project1");

        let req = TestRequest::with_uri("/aa-project1/path1").to_request();
        let res = srv.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_nested_scope() {
        let srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/t1").service(web::resource("/path1").to(HttpResponse::Created)),
        )))
        .await;

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_nested_scope_no_slash() {
        let srv =
            init_service(App::new().service(web::scope("/app").service(
                web::scope("t1").service(web::resource("/path1").to(HttpResponse::Created)),
            )))
            .await;

        let req = TestRequest::with_uri("/app/t1/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_nested_scope_root() {
        let srv = init_service(
            App::new().service(
                web::scope("/app").service(
                    web::scope("/t1")
                        .service(web::resource("").to(HttpResponse::Ok))
                        .service(web::resource("/").to(HttpResponse::Created)),
                ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/t1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/t1/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[actix_rt::test]
    async fn test_nested_scope_filter() {
        let srv = init_service(
            App::new().service(
                web::scope("/app").service(
                    web::scope("/t1")
                        .guard(guard::Get())
                        .service(web::resource("/path1").to(HttpResponse::Ok)),
                ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let req = TestRequest::with_uri("/app/t1/path1")
            .method(Method::GET)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_nested_scope_with_variable_segment() {
        let srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/{project_id}").service(web::resource("/path1").to(|r: HttpRequest| {
                HttpResponse::Created().body(format!("project: {}", &r.match_info()["project_id"]))
            })),
        )))
        .await;

        let req = TestRequest::with_uri("/app/project_1/path1").to_request();
        let res = srv.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        assert_body_eq!(res, b"project: project_1");
    }

    #[actix_rt::test]
    async fn test_nested2_scope_with_variable_segment() {
        let srv = init_service(App::new().service(web::scope("/app").service(
            web::scope("/{project}").service(web::scope("/{id}").service(
                web::resource("/path1").to(|r: HttpRequest| {
                    HttpResponse::Created().body(format!(
                        "project: {} - {}",
                        &r.match_info()["project"],
                        &r.match_info()["id"],
                    ))
                }),
            )),
        )))
        .await;

        let req = TestRequest::with_uri("/app/test/1/path1").to_request();
        let res = srv.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        assert_body_eq!(res, b"project: test - 1");

        let req = TestRequest::with_uri("/app/test/1/path2").to_request();
        let res = srv.call(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_default_resource() {
        let srv = init_service(
            App::new().service(
                web::scope("/app")
                    .service(web::resource("/path1").to(HttpResponse::Ok))
                    .default_service(|r: ServiceRequest| {
                        ok(r.into_response(HttpResponse::BadRequest()))
                    }),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/path2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/path2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_default_resource_propagation() {
        let srv = init_service(
            App::new()
                .service(web::scope("/app1").default_service(web::to(HttpResponse::BadRequest)))
                .service(web::scope("/app2"))
                .default_service(|r: ServiceRequest| {
                    ok(r.into_response(HttpResponse::MethodNotAllowed()))
                }),
        )
        .await;

        let req = TestRequest::with_uri("/non-exist").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/app1/non-exist").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_uri("/app2/non-exist").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[actix_rt::test]
    async fn test_middleware() {
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .wrap(
                        DefaultHeaders::new()
                            .add((header::CONTENT_TYPE, HeaderValue::from_static("0001"))),
                    )
                    .service(web::resource("/test").route(web::get().to(HttpResponse::Ok))),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[actix_rt::test]
    async fn test_middleware_body_type() {
        // Compile test that Scope accepts any body type; test for `EitherBody`
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .wrap_fn(|req, srv| {
                        let fut = srv.call(req);
                        async { Ok(fut.await?.map_into_right_body::<()>()) }
                    })
                    .service(web::resource("/test").route(web::get().to(|| async { "hello" }))),
            ),
        )
        .await;

        // test if `MessageBody::try_into_bytes()` is preserved across scope layer
        use actix_http::body::MessageBody as _;
        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&srv, req).await;
        let body = resp.into_body();
        assert_eq!(body.try_into_bytes().unwrap(), b"hello".as_ref());
    }

    #[actix_rt::test]
    async fn test_middleware_fn() {
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .wrap_fn(|req, srv| {
                        let fut = srv.call(req);
                        async move {
                            let mut res = fut.await?;
                            res.headers_mut()
                                .insert(header::CONTENT_TYPE, HeaderValue::from_static("0001"));
                            Ok(res)
                        }
                    })
                    .route("/test", web::get().to(HttpResponse::Ok)),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[actix_rt::test]
    async fn test_middleware_app_data() {
        let srv = init_service(
            App::new().service(
                web::scope("app")
                    .app_data(1usize)
                    .wrap_fn(|req, srv| {
                        assert_eq!(req.app_data::<usize>(), Some(&1usize));
                        req.extensions_mut().insert(1usize);
                        srv.call(req)
                    })
                    .route("/test", web::get().to(HttpResponse::Ok))
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

        let req = TestRequest::with_uri("/app/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/app/default").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // allow deprecated {App, Scope}::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_override_data() {
        let srv = init_service(App::new().data(1usize).service(
            web::scope("app").data(10usize).route(
                "/t",
                web::get().to(|data: web::Data<usize>| {
                    assert_eq!(**data, 10);
                    HttpResponse::Ok()
                }),
            ),
        ))
        .await;

        let req = TestRequest::with_uri("/app/t").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // allow deprecated `{App, Scope}::data`
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_override_data_default_service() {
        let srv =
            init_service(App::new().data(1usize).service(
                web::scope("app").data(10usize).default_service(web::to(
                    |data: web::Data<usize>| {
                        assert_eq!(**data, 10);
                        HttpResponse::Ok()
                    },
                )),
            ))
            .await;

        let req = TestRequest::with_uri("/app/t").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_override_app_data() {
        let srv = init_service(App::new().app_data(web::Data::new(1usize)).service(
            web::scope("app").app_data(web::Data::new(10usize)).route(
                "/t",
                web::get().to(|data: web::Data<usize>| {
                    assert_eq!(**data, 10);
                    HttpResponse::Ok()
                }),
            ),
        ))
        .await;

        let req = TestRequest::with_uri("/app/t").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_config() {
        let srv = init_service(App::new().service(web::scope("/app").configure(|s| {
            s.route("/path1", web::get().to(HttpResponse::Ok));
        })))
        .await;

        let req = TestRequest::with_uri("/app/path1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_scope_config_2() {
        let srv = init_service(App::new().service(web::scope("/app").configure(|s| {
            s.service(web::scope("/v1").configure(|s| {
                s.route("/", web::get().to(HttpResponse::Ok));
            }));
        })))
        .await;

        let req = TestRequest::with_uri("/app/v1/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_url_for_external() {
        let srv = init_service(App::new().service(web::scope("/app").configure(|s| {
            s.service(web::scope("/v1").configure(|s| {
                s.external_resource("youtube", "https://youtube.com/watch/{video_id}");
                s.route(
                    "/",
                    web::get().to(|req: HttpRequest| {
                        HttpResponse::Ok()
                            .body(req.url_for("youtube", ["xxxxxx"]).unwrap().to_string())
                    }),
                );
            }));
        })))
        .await;

        let req = TestRequest::with_uri("/app/v1/").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp).await;
        assert_eq!(body, &b"https://youtube.com/watch/xxxxxx"[..]);
    }

    #[actix_rt::test]
    async fn test_url_for_nested() {
        let srv = init_service(App::new().service(web::scope("/a").service(
            web::scope("/b").service(web::resource("/c/{stuff}").name("c").route(web::get().to(
                |req: HttpRequest| {
                    HttpResponse::Ok().body(format!("{}", req.url_for("c", ["12345"]).unwrap()))
                },
            ))),
        )))
        .await;

        let req = TestRequest::with_uri("/a/b/c/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp).await;
        assert_eq!(
            body,
            Bytes::from_static(b"http://localhost:8080/a/b/c/12345")
        );
    }

    #[actix_rt::test]
    async fn dynamic_scopes() {
        let srv = init_service(
            App::new().service(
                web::scope("/{a}/").service(
                    web::scope("/{b}/")
                        .route("", web::get().to(|_: HttpRequest| HttpResponse::Created()))
                        .route(
                            "/",
                            web::get().to(|_: HttpRequest| HttpResponse::Accepted()),
                        )
                        .route("/{c}", web::get().to(|_: HttpRequest| HttpResponse::Ok())),
                ),
            ),
        )
        .await;

        // note the unintuitive behavior with trailing slashes on scopes with dynamic segments
        let req = TestRequest::with_uri("/a//b//c").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/a//b/").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/a//b//").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let req = TestRequest::with_uri("/a//b//c/d").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let srv = init_service(
            App::new().service(
                web::scope("/{a}").service(
                    web::scope("/{b}")
                        .route("", web::get().to(|_: HttpRequest| HttpResponse::Created()))
                        .route(
                            "/",
                            web::get().to(|_: HttpRequest| HttpResponse::Accepted()),
                        )
                        .route("/{c}", web::get().to(|_: HttpRequest| HttpResponse::Ok())),
                ),
            ),
        )
        .await;

        let req = TestRequest::with_uri("/a/b/c").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/a/b").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/a/b/").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let req = TestRequest::with_uri("/a/b/c/d").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}

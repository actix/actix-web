use std::{cell::RefCell, fmt, future::Future, rc::Rc};

use actix_http::{body::MessageBody, Extensions, Request};
use actix_service::{
    apply, apply_fn_factory, boxed, IntoServiceFactory, ServiceFactory, ServiceFactoryExt,
    Transform,
};
use futures_util::FutureExt as _;

use crate::{
    app_service::{AppEntry, AppInit, AppRoutingFactory},
    config::ServiceConfig,
    data::{Data, DataFactory, FnDataFactory},
    dev::ResourceDef,
    error::Error,
    resource::Resource,
    route::Route,
    service::{
        AppServiceFactory, BoxedHttpServiceFactory, HttpServiceFactory, ServiceFactoryWrapper,
        ServiceRequest, ServiceResponse,
    },
};

/// The top-level builder for an Actix Web application.
pub struct App<T> {
    endpoint: T,
    services: Vec<Box<dyn AppServiceFactory>>,
    default: Option<Rc<BoxedHttpServiceFactory>>,
    factory_ref: Rc<RefCell<Option<AppRoutingFactory>>>,
    data_factories: Vec<FnDataFactory>,
    external: Vec<ResourceDef>,
    extensions: Extensions,
}

impl App<AppEntry> {
    /// Create application builder. Application can be configured with a builder-like pattern.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let factory_ref = Rc::new(RefCell::new(None));

        App {
            endpoint: AppEntry::new(factory_ref.clone()),
            data_factories: Vec::new(),
            services: Vec::new(),
            default: None,
            factory_ref,
            external: Vec::new(),
            extensions: Extensions::new(),
        }
    }
}

impl<T> App<T>
where
    T: ServiceFactory<ServiceRequest, Config = (), Error = Error, InitError = ()>,
{
    /// Set application (root level) data.
    ///
    /// Application data stored with `App::app_data()` method is available through the
    /// [`HttpRequest::app_data`](crate::HttpRequest::app_data) method at runtime.
    ///
    /// # [`Data<T>`]
    /// Any [`Data<T>`] type added here can utilize its extractor implementation in handlers.
    /// Types not wrapped in `Data<T>` cannot use this extractor. See [its docs](Data<T>) for more
    /// about its usage and patterns.
    ///
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
    ///
    /// # Shared Mutable State
    /// [`HttpServer::new`](crate::HttpServer::new) accepts an application factory rather than an
    /// application instance; the factory closure is called on each worker thread independently.
    /// Therefore, if you want to share a data object between different workers, a shareable object
    /// needs to be created first, outside the `HttpServer::new` closure and cloned into it.
    /// [`Data<T>`] is an example of such a sharable object.
    ///
    /// ```ignore
    /// let counter = web::Data::new(AppStateWithCounter {
    ///     counter: Mutex::new(0),
    /// });
    ///
    /// HttpServer::new(move || {
    ///     // move counter object into the closure and clone for each worker
    ///
    ///     App::new()
    ///         .app_data(counter.clone())
    ///         .route("/", web::get().to(handler))
    /// })
    /// ```
    #[doc(alias = "manage")]
    pub fn app_data<U: 'static>(mut self, data: U) -> Self {
        self.extensions.insert(data);
        self
    }

    /// Add application (root) data after wrapping in `Data<T>`.
    ///
    /// Deprecated in favor of [`app_data`](Self::app_data).
    #[deprecated(since = "4.0.0", note = "Use `.app_data(Data::new(val))` instead.")]
    pub fn data<U: 'static>(self, data: U) -> Self {
        self.app_data(Data::new(data))
    }

    /// Add application data factory that resolves asynchronously.
    ///
    /// Data items are constructed during application initialization, before the server starts
    /// accepting requests.
    ///
    /// The returned data value `D` is wrapped as [`Data<D>`].
    pub fn data_factory<F, Out, D, E>(mut self, data: F) -> Self
    where
        F: Fn() -> Out + 'static,
        Out: Future<Output = Result<D, E>> + 'static,
        D: 'static,
        E: std::fmt::Debug,
    {
        self.data_factories.push(Box::new(move || {
            {
                let fut = data();
                async move {
                    match fut.await {
                        Err(err) => {
                            log::error!("Can not construct data instance: {err:?}");
                            Err(())
                        }
                        Ok(data) => {
                            let data: Box<dyn DataFactory> = Box::new(Data::new(data));
                            Ok(data)
                        }
                    }
                }
            }
            .boxed_local()
        }));

        self
    }

    /// Run external configuration as part of the application building
    /// process
    ///
    /// This function is useful for moving parts of configuration to a
    /// different module or even library. For example,
    /// some of the resource's configuration could be moved to different module.
    ///
    /// ```
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// // this function could be located in different module
    /// fn config(cfg: &mut web::ServiceConfig) {
    ///     cfg.service(web::resource("/test")
    ///         .route(web::get().to(|| HttpResponse::Ok()))
    ///         .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
    ///     );
    /// }
    ///
    /// App::new()
    ///     .configure(config)  // <- register resources
    ///     .route("/index.html", web::get().to(|| HttpResponse::Ok()));
    /// ```
    pub fn configure<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut ServiceConfig),
    {
        let mut cfg = ServiceConfig::new();

        f(&mut cfg);

        self.services.extend(cfg.services);
        self.external.extend(cfg.external);
        self.extensions.extend(cfg.app_data);

        if let Some(default) = cfg.default {
            self.default = Some(default);
        }

        self
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::service()` method.
    /// This method can be used multiple times with same path, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// async fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// let app = App::new()
    ///     .route("/test1", web::get().to(index))
    ///     .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()));
    /// ```
    pub fn route(self, path: &str, mut route: Route) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register HTTP service.
    ///
    /// Http service is any type that implements `HttpServiceFactory` trait.
    ///
    /// Actix Web provides several services implementations:
    ///
    /// * *Resource* is an entry in resource table which corresponds to requested URL.
    /// * *Scope* is a set of resources with common root path.
    pub fn service<F>(mut self, factory: F) -> Self
    where
        F: HttpServiceFactory + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Default service that is invoked when no matching resource could be found.
    ///
    /// You can use a [`Route`] as default service.
    ///
    /// If a default service is not registered, an empty `404 Not Found` response will be sent to
    /// the client instead.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// async fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// let app = App::new()
    ///     .service(web::resource("/index.html").route(web::get().to(index)))
    ///     .default_service(web::to(|| HttpResponse::NotFound()));
    /// ```
    pub fn default_service<F, U>(mut self, svc: F) -> Self
    where
        F: IntoServiceFactory<U, ServiceRequest>,
        U: ServiceFactory<ServiceRequest, Config = (), Response = ServiceResponse, Error = Error>
            + 'static,
        U::InitError: fmt::Debug,
    {
        let svc = svc
            .into_factory()
            .map_init_err(|e| log::error!("Can not construct default service: {:?}", e));

        self.default = Some(Rc::new(boxed::factory(svc)));

        self
    }

    /// Register an external resource.
    ///
    /// External resources are useful for URL generation purposes only
    /// and are never considered for matching at request time. Calls to
    /// `HttpRequest::url_for()` will work as expected.
    ///
    /// ```
    /// use actix_web::{web, App, HttpRequest, HttpResponse, Result};
    ///
    /// async fn index(req: HttpRequest) -> Result<HttpResponse> {
    ///     let url = req.url_for("youtube", &["asdlkjqme"])?;
    ///     assert_eq!(url.as_str(), "https://youtube.com/watch/asdlkjqme");
    ///     Ok(HttpResponse::Ok().into())
    /// }
    ///
    /// let app = App::new()
    ///     .service(web::resource("/index.html").route(
    ///         web::get().to(index)))
    ///     .external_resource("youtube", "https://youtube.com/watch/{video_id}");
    /// ```
    pub fn external_resource<N, U>(mut self, name: N, url: U) -> Self
    where
        N: AsRef<str>,
        U: AsRef<str>,
    {
        let mut rdef = ResourceDef::new(url.as_ref());
        rdef.set_name(name.as_ref());
        self.external.push(rdef);
        self
    }

    /// Registers an app-wide middleware.
    ///
    /// Registers middleware, in the form of a middleware component (type), that runs during
    /// inbound and/or outbound processing in the request life-cycle (request -> response),
    /// modifying request/response as necessary, across all requests managed by the `App`.
    ///
    /// Use middleware when you need to read or modify *every* request or response in some way.
    ///
    /// Middleware can be applied similarly to individual `Scope`s and `Resource`s.
    /// See [`Scope::wrap`](crate::Scope::wrap) and [`Resource::wrap`].
    ///
    /// For more info on middleware take a look at the [`middleware` module][crate::middleware].
    ///
    /// # Examples
    /// ```
    /// use actix_web::{middleware, web, App};
    ///
    /// async fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// let app = App::new()
    ///     .wrap(middleware::Logger::default())
    ///     .route("/index.html", web::get().to(index));
    /// ```
    #[doc(alias = "middleware")]
    #[doc(alias = "use")] // nodejs terminology
    pub fn wrap<M, B>(
        self,
        mw: M,
    ) -> App<
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
        App {
            endpoint: apply(mw, self.endpoint),
            data_factories: self.data_factories,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
            external: self.external,
            extensions: self.extensions,
        }
    }

    /// Registers an app-wide function middleware.
    ///
    /// `mw` is a closure that runs during inbound and/or outbound processing in the request
    /// life-cycle (request -> response), modifying request/response as necessary, across all
    /// requests handled by the `App`.
    ///
    /// Use middleware when you need to read or modify *every* request or response in some way.
    ///
    /// Middleware can also be applied to individual `Scope`s and `Resource`s.
    ///
    /// See [`App::wrap`] for details on how middlewares compose with each other.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{dev::Service as _, middleware, web, App};
    /// use actix_web::http::header::{CONTENT_TYPE, HeaderValue};
    ///
    /// async fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// let app = App::new()
    ///     .wrap_fn(|req, srv| {
    ///         let fut = srv.call(req);
    ///         async {
    ///             let mut res = fut.await?;
    ///             res.headers_mut()
    ///                 .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    ///             Ok(res)
    ///         }
    ///     })
    ///     .route("/index.html", web::get().to(index));
    /// ```
    #[doc(alias = "middleware")]
    #[doc(alias = "use")] // nodejs terminology
    pub fn wrap_fn<F, R, B>(
        self,
        mw: F,
    ) -> App<
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
        App {
            endpoint: apply_fn_factory(self.endpoint, mw),
            data_factories: self.data_factories,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
            external: self.external,
            extensions: self.extensions,
        }
    }
}

impl<T, B> IntoServiceFactory<AppInit<T, B>, Request> for App<T>
where
    T: ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        > + 'static,
    B: MessageBody,
{
    fn into_factory(self) -> AppInit<T, B> {
        AppInit {
            async_data_factories: self.data_factories.into_boxed_slice().into(),
            endpoint: self.endpoint,
            services: Rc::new(RefCell::new(self.services)),
            external: RefCell::new(self.external),
            default: self.default,
            factory_ref: self.factory_ref,
            extensions: RefCell::new(Some(self.extensions)),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service as _;
    use actix_utils::future::{err, ok};
    use bytes::Bytes;

    use super::*;
    use crate::{
        http::{
            header::{self, HeaderValue},
            Method, StatusCode,
        },
        middleware::DefaultHeaders,
        test::{call_service, init_service, read_body, try_init_service, TestRequest},
        web, HttpRequest, HttpResponse,
    };

    #[actix_rt::test]
    async fn test_default_resource() {
        let srv =
            init_service(App::new().service(web::resource("/test").to(HttpResponse::Ok))).await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let srv = init_service(
            App::new()
                .service(web::resource("/test").to(HttpResponse::Ok))
                .service(
                    web::resource("/test2")
                        .default_service(|r: ServiceRequest| {
                            ok(r.into_response(HttpResponse::Created()))
                        })
                        .route(web::get().to(HttpResponse::Ok)),
                )
                .default_service(|r: ServiceRequest| {
                    ok(r.into_response(HttpResponse::MethodNotAllowed()))
                }),
        )
        .await;

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/test2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test2")
            .method(Method::POST)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_data_factory() {
        let srv = init_service(
            App::new()
                .data_factory(|| ok::<_, ()>(10usize))
                .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok())),
        )
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let srv = init_service(
            App::new()
                .data_factory(|| ok::<_, ()>(10u32))
                .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok())),
        )
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_data_factory_errors() {
        let srv = try_init_service(
            App::new()
                .data_factory(|| err::<u32, _>(()))
                .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok())),
        )
        .await;

        assert!(srv.is_err());
    }

    #[actix_rt::test]
    async fn test_extension() {
        let srv = init_service(App::new().app_data(10usize).service(web::resource("/").to(
            |req: HttpRequest| {
                assert_eq!(*req.app_data::<usize>().unwrap(), 10);
                HttpResponse::Ok()
            },
        )))
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_wrap() {
        let srv = init_service(
            App::new()
                .wrap(
                    DefaultHeaders::new()
                        .add((header::CONTENT_TYPE, HeaderValue::from_static("0001"))),
                )
                .route("/test", web::get().to(HttpResponse::Ok)),
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
    async fn test_router_wrap() {
        let srv = init_service(
            App::new()
                .route("/test", web::get().to(HttpResponse::Ok))
                .wrap(
                    DefaultHeaders::new()
                        .add((header::CONTENT_TYPE, HeaderValue::from_static("0001"))),
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
    async fn test_wrap_fn() {
        let srv = init_service(
            App::new()
                .wrap_fn(|req, srv| {
                    let fut = srv.call(req);
                    async move {
                        let mut res = fut.await?;
                        res.headers_mut()
                            .insert(header::CONTENT_TYPE, HeaderValue::from_static("0001"));
                        Ok(res)
                    }
                })
                .service(web::resource("/test").to(HttpResponse::Ok)),
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
    async fn test_router_wrap_fn() {
        let srv = init_service(
            App::new()
                .route("/test", web::get().to(HttpResponse::Ok))
                .wrap_fn(|req, srv| {
                    let fut = srv.call(req);
                    async {
                        let mut res = fut.await?;
                        res.headers_mut()
                            .insert(header::CONTENT_TYPE, HeaderValue::from_static("0001"));
                        Ok(res)
                    }
                }),
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
    async fn test_external_resource() {
        let srv = init_service(
            App::new()
                .external_resource("youtube", "https://youtube.com/watch/{video_id}")
                .route(
                    "/test",
                    web::get().to(|req: HttpRequest| {
                        HttpResponse::Ok()
                            .body(req.url_for("youtube", ["12345"]).unwrap().to_string())
                    }),
                ),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"https://youtube.com/watch/12345"));
    }

    #[test]
    fn can_be_returned_from_fn() {
        /// compile-only test for returning app type from function
        pub fn my_app() -> App<
            impl ServiceFactory<
                ServiceRequest,
                Response = ServiceResponse<impl MessageBody>,
                Config = (),
                InitError = (),
                Error = Error,
            >,
        > {
            App::new()
                // logger can be removed without affecting the return type
                .wrap(crate::middleware::Logger::default())
                .route("/", web::to(|| async { "hello" }))
        }

        #[allow(clippy::let_underscore_future)]
        let _ = init_service(my_app());
    }
}

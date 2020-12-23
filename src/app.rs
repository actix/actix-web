use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody};
use actix_http::Extensions;
use actix_service::boxed::{self, BoxServiceFactory};
use actix_service::{
    apply, apply_fn_factory, IntoServiceFactory, ServiceFactory, Transform,
};
use futures_util::future::FutureExt;

use crate::app_service::{AppEntry, AppInit, AppRoutingFactory};
use crate::config::ServiceConfig;
use crate::data::{Data, DataFactory, FnDataFactory};
use crate::dev::ResourceDef;
use crate::error::Error;
use crate::resource::Resource;
use crate::route::Route;
use crate::service::{
    AppServiceFactory, HttpServiceFactory, ServiceFactoryWrapper, ServiceRequest,
    ServiceResponse,
};

type HttpNewService = BoxServiceFactory<(), ServiceRequest, ServiceResponse, Error, ()>;

/// Application builder - structure that follows the builder pattern
/// for building application instances.
pub struct App<T, B> {
    endpoint: T,
    services: Vec<Box<dyn AppServiceFactory>>,
    default: Option<Rc<HttpNewService>>,
    factory_ref: Rc<RefCell<Option<AppRoutingFactory>>>,
    data: Vec<Box<dyn DataFactory>>,
    data_factories: Vec<FnDataFactory>,
    external: Vec<ResourceDef>,
    extensions: Extensions,
    _t: PhantomData<B>,
}

impl App<AppEntry, Body> {
    /// Create application builder. Application can be configured with a builder-like pattern.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let fref = Rc::new(RefCell::new(None));
        App {
            endpoint: AppEntry::new(fref.clone()),
            data: Vec::new(),
            data_factories: Vec::new(),
            services: Vec::new(),
            default: None,
            factory_ref: fref,
            external: Vec::new(),
            extensions: Extensions::new(),
            _t: PhantomData,
        }
    }
}

impl<T, B> App<T, B>
where
    B: MessageBody,
    T: ServiceFactory<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    /// Set application data. Application data could be accessed
    /// by using `Data<T>` extractor where `T` is data type.
    ///
    /// **Note**: http server accepts an application factory rather than
    /// an application instance. Http server constructs an application
    /// instance for each thread, thus application data must be constructed
    /// multiple times. If you want to share data between different
    /// threads, a shared object should be used, e.g. `Arc`. Internally `Data` type
    /// uses `Arc` so data could be created outside of app factory and clones could
    /// be stored via `App::app_data()` method.
    ///
    /// ```rust
    /// use std::cell::Cell;
    /// use actix_web::{web, App, HttpResponse, Responder};
    ///
    /// struct MyData {
    ///     counter: Cell<usize>,
    /// }
    ///
    /// async fn index(data: web::Data<MyData>) -> impl Responder {
    ///     data.counter.set(data.counter.get() + 1);
    ///     HttpResponse::Ok()
    /// }
    ///
    /// let app = App::new()
    ///     .data(MyData{ counter: Cell::new(0) })
    ///     .service(
    ///         web::resource("/index.html").route(
    ///             web::get().to(index)));
    /// ```
    pub fn data<U: 'static>(mut self, data: U) -> Self {
        self.data.push(Box::new(Data::new(data)));
        self
    }

    /// Set application data factory. This function is
    /// similar to `.data()` but it accepts data factory. Data object get
    /// constructed asynchronously during application initialization.
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
                        Err(e) => {
                            log::error!("Can not construct data instance: {:?}", e);
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

    /// Set application level arbitrary data item.
    ///
    /// Application data stored with `App::app_data()` method is available
    /// via `HttpRequest::app_data()` method at runtime.
    ///
    /// This method could be used for storing `Data<T>` as well, in that case
    /// data could be accessed by using `Data<T>` extractor.
    pub fn app_data<U: 'static>(mut self, ext: U) -> Self {
        self.extensions.insert(ext);
        self
    }

    /// Run external configuration as part of the application building
    /// process
    ///
    /// This function is useful for moving parts of configuration to a
    /// different module or even library. For example,
    /// some of the resource's configuration could be moved to different module.
    ///
    /// ```rust
    /// # extern crate actix_web;
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
    /// fn main() {
    ///     let app = App::new()
    ///         .wrap(middleware::Logger::default())
    ///         .configure(config)  // <- register resources
    ///         .route("/index.html", web::get().to(|| HttpResponse::Ok()));
    /// }
    /// ```
    pub fn configure<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut ServiceConfig),
    {
        let mut cfg = ServiceConfig::new();
        f(&mut cfg);
        self.data.extend(cfg.data);
        self.services.extend(cfg.services);
        self.external.extend(cfg.external);
        self.extensions.extend(cfg.extensions);
        self
    }

    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::service()` method.
    /// This method can be used multiple times with same path, in that case
    /// multiple resources with one route would be registered for same resource path.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// async fn index(data: web::Path<(String, String)>) -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .route("/test1", web::get().to(index))
    ///         .route("/test2", web::post().to(|| HttpResponse::MethodNotAllowed()));
    /// }
    /// ```
    pub fn route(self, path: &str, mut route: Route) -> Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register http service.
    ///
    /// Http service is any type that implements `HttpServiceFactory` trait.
    ///
    /// Actix web provides several services implementations:
    ///
    /// * *Resource* is an entry in resource table which corresponds to requested URL.
    /// * *Scope* is a set of resources with common root path.
    /// * "StaticFiles" is a service for static files support
    pub fn service<F>(mut self, factory: F) -> Self
    where
        F: HttpServiceFactory + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Default service to be used if no matching resource could be found.
    ///
    /// It is possible to use services like `Resource`, `Route`.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// async fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .service(
    ///             web::resource("/index.html").route(web::get().to(index)))
    ///         .default_service(
    ///             web::route().to(|| HttpResponse::NotFound()));
    /// }
    /// ```
    ///
    /// It is also possible to use static files as default service.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .service(
    ///             web::resource("/index.html").to(|| HttpResponse::Ok()))
    ///         .default_service(
    ///             web::to(|| HttpResponse::NotFound())
    ///         );
    /// }
    /// ```
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
        self.default = Some(Rc::new(boxed::factory(f.into_factory().map_init_err(
            |e| log::error!("Can not construct default service: {:?}", e),
        ))));

        self
    }

    /// Register an external resource.
    ///
    /// External resources are useful for URL generation purposes only
    /// and are never considered for matching at request time. Calls to
    /// `HttpRequest::url_for()` will work as expected.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpRequest, HttpResponse, Result};
    ///
    /// async fn index(req: HttpRequest) -> Result<HttpResponse> {
    ///     let url = req.url_for("youtube", &["asdlkjqme"])?;
    ///     assert_eq!(url.as_str(), "https://youtube.com/watch/asdlkjqme");
    ///     Ok(HttpResponse::Ok().into())
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .service(web::resource("/index.html").route(
    ///             web::get().to(index)))
    ///         .external_resource("youtube", "https://youtube.com/watch/{video_id}");
    /// }
    /// ```
    pub fn external_resource<N, U>(mut self, name: N, url: U) -> Self
    where
        N: AsRef<str>,
        U: AsRef<str>,
    {
        let mut rdef = ResourceDef::new(url.as_ref());
        *rdef.name_mut() = name.as_ref().to_string();
        self.external.push(rdef);
        self
    }

    /// Registers middleware, in the form of a middleware component (type),
    /// that runs during inbound and/or outbound processing in the request
    /// life-cycle (request -> response), modifying request/response as
    /// necessary, across all requests managed by the *Application*.
    ///
    /// Use middleware when you need to read or modify *every* request or
    /// response in some way.
    ///
    /// Notice that the keyword for registering middleware is `wrap`. As you
    /// register middleware using `wrap` in the App builder,  imagine wrapping
    /// layers around an inner App.  The first middleware layer exposed to a
    /// Request is the outermost layer-- the *last* registered in
    /// the builder chain.  Consequently, the *first* middleware registered
    /// in the builder chain is the *last* to execute during request processing.
    ///
    /// ```rust
    /// use actix_service::Service;
    /// use actix_web::{middleware, web, App};
    /// use actix_web::http::{header::CONTENT_TYPE, HeaderValue};
    ///
    /// async fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .wrap(middleware::Logger::default())
    ///         .route("/index.html", web::get().to(index));
    /// }
    /// ```
    pub fn wrap<M, B1>(
        self,
        mw: M,
    ) -> App<
        impl ServiceFactory<
            Config = (),
            Request = ServiceRequest,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
        B1,
    >
    where
        M: Transform<
            T::Service,
            Request = ServiceRequest,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
        B1: MessageBody,
    {
        App {
            endpoint: apply(mw, self.endpoint),
            data: self.data,
            data_factories: self.data_factories,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
            external: self.external,
            extensions: self.extensions,
            _t: PhantomData,
        }
    }

    /// Registers middleware, in the form of a closure, that runs during inbound
    /// and/or outbound processing in the request life-cycle (request -> response),
    /// modifying request/response as necessary, across all requests managed by
    /// the *Application*.
    ///
    /// Use middleware when you need to read or modify *every* request or response in some way.
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
    ///     let app = App::new()
    ///         .wrap_fn(|req, srv| {
    ///             let fut = srv.call(req);
    ///             async {
    ///                 let mut res = fut.await?;
    ///                 res.headers_mut().insert(
    ///                    CONTENT_TYPE, HeaderValue::from_static("text/plain"),
    ///                 );
    ///                 Ok(res)
    ///             }
    ///         })
    ///         .route("/index.html", web::get().to(index));
    /// }
    /// ```
    pub fn wrap_fn<B1, F, R>(
        self,
        mw: F,
    ) -> App<
        impl ServiceFactory<
            Config = (),
            Request = ServiceRequest,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
        B1,
    >
    where
        B1: MessageBody,
        F: FnMut(ServiceRequest, &mut T::Service) -> R + Clone,
        R: Future<Output = Result<ServiceResponse<B1>, Error>>,
    {
        App {
            endpoint: apply_fn_factory(self.endpoint, mw),
            data: self.data,
            data_factories: self.data_factories,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
            external: self.external,
            extensions: self.extensions,
            _t: PhantomData,
        }
    }
}

impl<T, B> IntoServiceFactory<AppInit<T, B>> for App<T, B>
where
    B: MessageBody,
    T: ServiceFactory<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    fn into_factory(self) -> AppInit<T, B> {
        AppInit {
            data: self.data.into_boxed_slice().into(),
            data_factories: self.data_factories.into_boxed_slice().into(),
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
    use actix_service::Service;
    use bytes::Bytes;
    use futures_util::future::{err, ok};

    use super::*;
    use crate::http::{header, HeaderValue, Method, StatusCode};
    use crate::middleware::DefaultHeaders;
    use crate::service::ServiceRequest;
    use crate::test::{
        call_service, init_service, read_body, try_init_service, TestRequest,
    };
    use crate::{web, HttpRequest, HttpResponse};

    #[actix_rt::test]
    async fn test_default_resource() {
        let mut srv = init_service(
            App::new().service(web::resource("/test").to(HttpResponse::Ok)),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = init_service(
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

    #[actix_rt::test]
    async fn test_data_factory() {
        let mut srv =
            init_service(App::new().data_factory(|| ok::<_, ()>(10usize)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ))
            .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data_factory(|| ok::<_, ()>(10u32)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ))
            .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[actix_rt::test]
    async fn test_data_factory_errors() {
        let srv =
            try_init_service(App::new().data_factory(|| err::<u32, _>(())).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ))
            .await;

        assert!(srv.is_err());
    }

    #[actix_rt::test]
    async fn test_extension() {
        let mut srv = init_service(App::new().app_data(10usize).service(
            web::resource("/").to(|req: HttpRequest| {
                assert_eq!(*req.app_data::<usize>().unwrap(), 10);
                HttpResponse::Ok()
            }),
        ))
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_wrap() {
        let mut srv = init_service(
            App::new()
                .wrap(
                    DefaultHeaders::new()
                        .header(header::CONTENT_TYPE, HeaderValue::from_static("0001")),
                )
                .route("/test", web::get().to(HttpResponse::Ok)),
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
    async fn test_router_wrap() {
        let mut srv = init_service(
            App::new()
                .route("/test", web::get().to(HttpResponse::Ok))
                .wrap(
                    DefaultHeaders::new()
                        .header(header::CONTENT_TYPE, HeaderValue::from_static("0001")),
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
    async fn test_wrap_fn() {
        let mut srv = init_service(
            App::new()
                .wrap_fn(|req, srv| {
                    let fut = srv.call(req);
                    async move {
                        let mut res = fut.await?;
                        res.headers_mut().insert(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("0001"),
                        );
                        Ok(res)
                    }
                })
                .service(web::resource("/test").to(HttpResponse::Ok)),
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
    async fn test_router_wrap_fn() {
        let mut srv = init_service(
            App::new()
                .route("/test", web::get().to(HttpResponse::Ok))
                .wrap_fn(|req, srv| {
                    let fut = srv.call(req);
                    async {
                        let mut res = fut.await?;
                        res.headers_mut().insert(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("0001"),
                        );
                        Ok(res)
                    }
                }),
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
    async fn test_external_resource() {
        let mut srv = init_service(
            App::new()
                .external_resource("youtube", "https://youtube.com/watch/{video_id}")
                .route(
                    "/test",
                    web::get().to(|req: HttpRequest| {
                        HttpResponse::Ok().body(
                            req.url_for("youtube", &["12345"]).unwrap().to_string(),
                        )
                    }),
                ),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"https://youtube.com/watch/12345"));
    }
}

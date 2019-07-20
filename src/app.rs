use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody};
use actix_service::boxed::{self, BoxedNewService};
use actix_service::{
    apply_transform, IntoNewService, IntoTransform, NewService, Transform,
};
use futures::{Future, IntoFuture};

use crate::app_service::{AppEntry, AppInit, AppRoutingFactory};
use crate::config::{AppConfig, AppConfigInner, ServiceConfig};
use crate::data::{Data, DataFactory};
use crate::dev::ResourceDef;
use crate::error::Error;
use crate::resource::Resource;
use crate::route::Route;
use crate::service::{
    HttpServiceFactory, ServiceFactory, ServiceFactoryWrapper, ServiceRequest,
    ServiceResponse,
};

type HttpNewService = BoxedNewService<(), ServiceRequest, ServiceResponse, Error, ()>;
type FnDataFactory =
    Box<dyn Fn() -> Box<dyn Future<Item = Box<dyn DataFactory>, Error = ()>>>;

/// Application builder - structure that follows the builder pattern
/// for building application instances.
pub struct App<T, B> {
    endpoint: T,
    services: Vec<Box<dyn ServiceFactory>>,
    default: Option<Rc<HttpNewService>>,
    factory_ref: Rc<RefCell<Option<AppRoutingFactory>>>,
    data: Vec<Box<dyn DataFactory>>,
    data_factories: Vec<FnDataFactory>,
    config: AppConfigInner,
    external: Vec<ResourceDef>,
    _t: PhantomData<(B)>,
}

impl App<AppEntry, Body> {
    /// Create application builder. Application can be configured with a builder-like pattern.
    pub fn new() -> Self {
        let fref = Rc::new(RefCell::new(None));
        App {
            endpoint: AppEntry::new(fref.clone()),
            data: Vec::new(),
            data_factories: Vec::new(),
            services: Vec::new(),
            default: None,
            factory_ref: fref,
            config: AppConfigInner::default(),
            external: Vec::new(),
            _t: PhantomData,
        }
    }
}

impl<T, B> App<T, B>
where
    B: MessageBody,
    T: NewService<
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
    /// threads, a shared object should be used, e.g. `Arc`. Application
    /// data does not need to be `Send` or `Sync`.
    ///
    /// ```rust
    /// use std::cell::Cell;
    /// use actix_web::{web, App};
    ///
    /// struct MyData {
    ///     counter: Cell<usize>,
    /// }
    ///
    /// fn index(data: web::Data<MyData>) {
    ///     data.counter.set(data.counter.get() + 1);
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .data(MyData{ counter: Cell::new(0) })
    ///         .service(
    ///             web::resource("/index.html").route(
    ///                 web::get().to(index)));
    /// }
    /// ```
    pub fn data<U: 'static>(mut self, data: U) -> Self {
        self.data.push(Box::new(Data::new(data)));
        self
    }

    /// Set application data factory. This function is
    /// similar to `.data()` but it accepts data factory. Data object get
    /// constructed asynchronously during application initialization.
    pub fn data_factory<F, Out>(mut self, data: F) -> Self
    where
        F: Fn() -> Out + 'static,
        Out: IntoFuture + 'static,
        Out::Error: std::fmt::Debug,
    {
        self.data_factories.push(Box::new(move || {
            Box::new(
                data()
                    .into_future()
                    .map_err(|e| {
                        log::error!("Can not construct data instance: {:?}", e);
                    })
                    .map(|data| {
                        let data: Box<dyn DataFactory> = Box::new(Data::new(data));
                        data
                    }),
            )
        }));
        self
    }

    /// Set application data. Application data could be accessed
    /// by using `Data<T>` extractor where `T` is data type.
    pub fn register_data<U: 'static>(mut self, data: Data<U>) -> Self {
        self.data.push(Box::new(data));
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
    /// fn index(data: web::Path<(String, String)>) -> &'static str {
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

    /// Set server host name.
    ///
    /// Host name is used by application router as a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn hostname(mut self, val: &str) -> Self {
        self.config.host = val.to_owned();
        self
    }

    /// Default service to be used if no matching resource could be found.
    ///
    /// It is possible to use services like `Resource`, `Route`.
    ///
    /// ```rust
    /// use actix_web::{web, App, HttpResponse};
    ///
    /// fn index() -> &'static str {
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
        F: IntoNewService<U>,
        U: NewService<
                Config = (),
                Request = ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
            > + 'static,
        U::InitError: fmt::Debug,
    {
        // create and configure default resource
        self.default = Some(Rc::new(boxed::new_service(
            f.into_new_service().map_init_err(|e| {
                log::error!("Can not construct default service: {:?}", e)
            }),
        )));

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
    /// fn index(req: HttpRequest) -> Result<HttpResponse> {
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
    /// lifecycle (request -> response), modifying request/response as
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
    /// # use futures::Future;
    /// use actix_web::{middleware, web, App};
    /// use actix_web::http::{header::CONTENT_TYPE, HeaderValue};
    ///
    /// fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .wrap(middleware::Logger::default())
    ///         .route("/index.html", web::get().to(index));
    /// }
    /// ```
    pub fn wrap<M, B1, F>(
        self,
        mw: F,
    ) -> App<
        impl NewService<
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
        F: IntoTransform<M, T::Service>,
    {
        let endpoint = apply_transform(mw, self.endpoint);
        App {
            endpoint,
            data: self.data,
            data_factories: self.data_factories,
            services: self.services,
            default: self.default,
            factory_ref: self.factory_ref,
            config: self.config,
            external: self.external,
            _t: PhantomData,
        }
    }

    /// Registers middleware, in the form of a closure, that runs during inbound
    /// and/or outbound processing in the request lifecycle (request -> response),
    /// modifying request/response as necessary, across all requests managed by
    /// the *Application*.
    ///
    /// Use middleware when you need to read or modify *every* request or response in some way.
    ///
    /// ```rust
    /// use actix_service::Service;
    /// # use futures::Future;
    /// use actix_web::{web, App};
    /// use actix_web::http::{header::CONTENT_TYPE, HeaderValue};
    ///
    /// fn index() -> &'static str {
    ///     "Welcome!"
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .wrap_fn(|req, srv|
    ///             srv.call(req).map(|mut res| {
    ///                 res.headers_mut().insert(
    ///                    CONTENT_TYPE, HeaderValue::from_static("text/plain"),
    ///                 );
    ///                 res
    ///             }))
    ///         .route("/index.html", web::get().to(index));
    /// }
    /// ```
    pub fn wrap_fn<B1, F, R>(
        self,
        mw: F,
    ) -> App<
        impl NewService<
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
        R: IntoFuture<Item = ServiceResponse<B1>, Error = Error>,
    {
        self.wrap(mw)
    }
}

impl<T, B> IntoNewService<AppInit<T, B>> for App<T, B>
where
    B: MessageBody,
    T: NewService<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    fn into_new_service(self) -> AppInit<T, B> {
        AppInit {
            data: Rc::new(self.data),
            data_factories: Rc::new(self.data_factories),
            endpoint: self.endpoint,
            services: Rc::new(RefCell::new(self.services)),
            external: RefCell::new(self.external),
            default: self.default,
            factory_ref: self.factory_ref,
            config: RefCell::new(AppConfig(Rc::new(self.config))),
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service;
    use bytes::Bytes;
    use futures::{Future, IntoFuture};

    use super::*;
    use crate::http::{header, HeaderValue, Method, StatusCode};
    use crate::service::{ServiceRequest, ServiceResponse};
    use crate::test::{
        block_fn, block_on, call_service, init_service, read_body, TestRequest,
    };
    use crate::{web, Error, HttpRequest, HttpResponse};

    #[test]
    fn test_default_resource() {
        let mut srv = init_service(
            App::new().service(web::resource("/test").to(|| HttpResponse::Ok())),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = block_fn(|| srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = init_service(
            App::new()
                .service(web::resource("/test").to(|| HttpResponse::Ok()))
                .service(
                    web::resource("/test2")
                        .default_service(|r: ServiceRequest| {
                            r.into_response(HttpResponse::Created())
                        })
                        .route(web::get().to(|| HttpResponse::Ok())),
                )
                .default_service(|r: ServiceRequest| {
                    r.into_response(HttpResponse::MethodNotAllowed())
                }),
        );

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let req = TestRequest::with_uri("/test2").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/test2")
            .method(Method::POST)
            .to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[test]
    fn test_data_factory() {
        let mut srv =
            init_service(App::new().data_factory(|| Ok::<_, ()>(10usize)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mut srv =
            init_service(App::new().data_factory(|| Ok::<_, ()>(10u32)).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    fn md<S, B>(
        req: ServiceRequest,
        srv: &mut S,
    ) -> impl IntoFuture<Item = ServiceResponse<B>, Error = Error>
    where
        S: Service<
            Request = ServiceRequest,
            Response = ServiceResponse<B>,
            Error = Error,
        >,
    {
        srv.call(req).map(|mut res| {
            res.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("0001"));
            res
        })
    }

    #[test]
    fn test_wrap() {
        let mut srv = init_service(
            App::new()
                .wrap(md)
                .route("/test", web::get().to(|| HttpResponse::Ok())),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[test]
    fn test_router_wrap() {
        let mut srv = init_service(
            App::new()
                .route("/test", web::get().to(|| HttpResponse::Ok()))
                .wrap(md),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[test]
    fn test_wrap_fn() {
        let mut srv = init_service(
            App::new()
                .wrap_fn(|req, srv| {
                    srv.call(req).map(|mut res| {
                        res.headers_mut().insert(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("0001"),
                        );
                        res
                    })
                })
                .service(web::resource("/test").to(|| HttpResponse::Ok())),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[test]
    fn test_router_wrap_fn() {
        let mut srv = init_service(
            App::new()
                .route("/test", web::get().to(|| HttpResponse::Ok()))
                .wrap_fn(|req, srv| {
                    srv.call(req).map(|mut res| {
                        res.headers_mut().insert(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("0001"),
                        );
                        res
                    })
                }),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }

    #[test]
    fn test_external_resource() {
        let mut srv = init_service(
            App::new()
                .external_resource("youtube", "https://youtube.com/watch/{video_id}")
                .route(
                    "/test",
                    web::get().to(|req: HttpRequest| {
                        HttpResponse::Ok().body(format!(
                            "{}",
                            req.url_for("youtube", &["12345"]).unwrap()
                        ))
                    }),
                ),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp);
        assert_eq!(body, Bytes::from_static(b"https://youtube.com/watch/12345"));
    }
}

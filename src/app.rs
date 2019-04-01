use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody};
#[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
use actix_http::encoding::{Decoder, Encoder};
use actix_server_config::ServerConfig;
use actix_service::boxed::{self, BoxedNewService};
use actix_service::{
    apply_transform, IntoNewService, IntoTransform, NewService, Transform,
};
#[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
use bytes::Bytes;
use futures::{IntoFuture, Stream};

use crate::app_service::{AppChain, AppEntry, AppInit, AppRouting, AppRoutingFactory};
use crate::config::{AppConfig, AppConfigInner};
use crate::data::{Data, DataFactory};
use crate::dev::{Payload, PayloadStream, ResourceDef};
use crate::error::{Error, PayloadError};
use crate::resource::Resource;
use crate::route::Route;
use crate::service::{
    HttpServiceFactory, ServiceFactory, ServiceFactoryWrapper, ServiceRequest,
    ServiceResponse,
};

type HttpNewService<P> =
    BoxedNewService<(), ServiceRequest<P>, ServiceResponse, Error, ()>;

/// Application builder - structure that follows the builder pattern
/// for building application instances.
pub struct App<In, Out, T>
where
    T: NewService<Request = ServiceRequest<In>, Response = ServiceRequest<Out>>,
{
    chain: T,
    data: Vec<Box<DataFactory>>,
    config: AppConfigInner,
    _t: PhantomData<(In, Out)>,
}

impl App<PayloadStream, PayloadStream, AppChain> {
    /// Create application builder. Application can be configured with a builder-like pattern.
    pub fn new() -> Self {
        App {
            chain: AppChain,
            data: Vec::new(),
            config: AppConfigInner::default(),
            _t: PhantomData,
        }
    }
}

impl<In, Out, T> App<In, Out, T>
where
    In: 'static,
    Out: 'static,
    T: NewService<
        Request = ServiceRequest<In>,
        Response = ServiceRequest<Out>,
        Error = Error,
        InitError = (),
    >,
{
    /// Set application data. Applicatin data could be accessed
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
    pub fn data<S: 'static>(mut self, data: S) -> Self {
        self.data.push(Box::new(Data::new(data)));
        self
    }

    /// Set application data factory. This function is
    /// similar to `.data()` but it accepts data factory. Data object get
    /// constructed asynchronously during application initialization.
    pub fn data_factory<F, R>(mut self, data: F) -> Self
    where
        F: Fn() -> R + 'static,
        R: IntoFuture + 'static,
        R::Error: std::fmt::Debug,
    {
        self.data.push(Box::new(data));
        self
    }

    /// Registers middleware, in the form of a middleware component (type), 
    /// that runs during inbound and/or outbound processing in the request 
    /// lifecycle (request -> response), modifying request/response as 
    /// necessary, across all requests managed by the *Application*.
    ///
    /// Use middleware when you need to read or modify *every* request or response in some way.
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
    pub fn wrap<M, B, F>(
        self,
        mw: F,
    ) -> AppRouter<
        T,
        Out,
        B,
        impl NewService<
            Request = ServiceRequest<Out>,
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
            AppRouting<Out>,
            Request = ServiceRequest<Out>,
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        >,
        F: IntoTransform<M, AppRouting<Out>>,
    {
        let fref = Rc::new(RefCell::new(None));
        let endpoint = apply_transform(mw, AppEntry::new(fref.clone()));
        AppRouter {
            endpoint,
            chain: self.chain,
            data: self.data,
            services: Vec::new(),
            default: None,
            factory_ref: fref,
            config: self.config,
            external: Vec::new(),
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
    pub fn wrap_fn<F, R, B>(
        self,
        mw: F,
    ) -> AppRouter<
        T,
        Out,
        B,
        impl NewService<
            Request = ServiceRequest<Out>,
            Response = ServiceResponse<B>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        F: FnMut(ServiceRequest<Out>, &mut AppRouting<Out>) -> R + Clone,
        R: IntoFuture<Item = ServiceResponse<B>, Error = Error>,
    {
        self.wrap(mw)
    }

    /// Register a request modifier. It can modify any request parameters
    /// including request payload type.
    pub fn chain<C, F, P>(
        self,
        chain: F,
    ) -> App<
        In,
        P,
        impl NewService<
            Request = ServiceRequest<In>,
            Response = ServiceRequest<P>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        C: NewService<
            Request = ServiceRequest<Out>,
            Response = ServiceRequest<P>,
            Error = Error,
            InitError = (),
        >,
        F: IntoNewService<C>,
    {
        let chain = self.chain.and_then(chain.into_new_service());
        App {
            chain,
            data: self.data,
            config: self.config,
            _t: PhantomData,
        }
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
    pub fn route(
        self,
        path: &str,
        mut route: Route<Out>,
    ) -> AppRouter<T, Out, Body, AppEntry<Out>> {
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
    pub fn service<F>(self, service: F) -> AppRouter<T, Out, Body, AppEntry<Out>>
    where
        F: HttpServiceFactory<Out> + 'static,
    {
        let fref = Rc::new(RefCell::new(None));

        AppRouter {
            chain: self.chain,
            default: None,
            endpoint: AppEntry::new(fref.clone()),
            factory_ref: fref,
            data: self.data,
            config: self.config,
            services: vec![Box::new(ServiceFactoryWrapper::new(service))],
            external: Vec::new(),
            _t: PhantomData,
        }
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

    #[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
    /// Enable content compression and decompression.
    pub fn enable_encoding(
        self,
    ) -> AppRouter<
        impl NewService<
            Request = ServiceRequest<In>,
            Response = ServiceRequest<Decoder<Payload<Out>>>,
            Error = Error,
            InitError = (),
        >,
        Decoder<Payload<Out>>,
        Encoder<Body>,
        impl NewService<
            Request = ServiceRequest<Decoder<Payload<Out>>>,
            Response = ServiceResponse<Encoder<Body>>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        Out: Stream<Item = Bytes, Error = PayloadError>,
    {
        use crate::middleware::encoding::{Compress, Decompress};

        self.chain(Decompress::new()).wrap(Compress::default())
    }
}

/// Application router builder - Structure that follows the builder pattern
/// for building application instances.
pub struct AppRouter<C, P, B, T> {
    chain: C,
    endpoint: T,
    services: Vec<Box<ServiceFactory<P>>>,
    default: Option<Rc<HttpNewService<P>>>,
    factory_ref: Rc<RefCell<Option<AppRoutingFactory<P>>>>,
    data: Vec<Box<DataFactory>>,
    config: AppConfigInner,
    external: Vec<ResourceDef>,
    _t: PhantomData<(P, B)>,
}

impl<C, P, B, T> AppRouter<C, P, B, T>
where
    P: 'static,
    B: MessageBody,
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
{
    /// Configure route for a specific path.
    ///
    /// This is a simplified version of the `App::service()` method.
    /// This method can not be could multiple times, in that case
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
    pub fn route(self, path: &str, mut route: Route<P>) -> Self {
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
        F: HttpServiceFactory<P> + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Registers middleware, in the form of a middleware component (type), 
    /// that runs during inbound and/or outbound processing in the request 
    /// lifecycle (request -> response), modifying request/response as 
    /// necessary, across all requests managed by the *Route*.
    ///
    /// Use middleware when you need to read or modify *every* request or response in some way.
    ///
    pub fn wrap<M, B1, F>(
        self,
        mw: F,
    ) -> AppRouter<
        C,
        P,
        B1,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        M: Transform<
            T::Service,
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
        B1: MessageBody,
        F: IntoTransform<M, T::Service>,
    {
        let endpoint = apply_transform(mw, self.endpoint);
        AppRouter {
            endpoint,
            chain: self.chain,
            data: self.data,
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
    /// the *Route*.
    ///
    /// Use middleware when you need to read or modify *every* request or response in some way.
    ///
    pub fn wrap_fn<B1, F, R>(
        self,
        mw: F,
    ) -> AppRouter<
        C,
        P,
        B1,
        impl NewService<
            Request = ServiceRequest<P>,
            Response = ServiceResponse<B1>,
            Error = Error,
            InitError = (),
        >,
    >
    where
        B1: MessageBody,
        F: FnMut(ServiceRequest<P>, &mut T::Service) -> R + Clone,
        R: IntoFuture<Item = ServiceResponse<B1>, Error = Error>,
    {
        self.wrap(mw)
    }

    /// Default resource to be used if no matching resource could be found.
    pub fn default_resource<F, U>(mut self, f: F) -> Self
    where
        F: FnOnce(Resource<P>) -> Resource<P, U>,
        U: NewService<
                Request = ServiceRequest<P>,
                Response = ServiceResponse,
                Error = Error,
                InitError = (),
            > + 'static,
    {
        // create and configure default resource
        self.default = Some(Rc::new(boxed::new_service(
            f(Resource::new("")).into_new_service().map_init_err(|_| ()),
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
}

impl<C, T, P: 'static, B: MessageBody> IntoNewService<AppInit<C, T, P, B>, ServerConfig>
    for AppRouter<C, P, B, T>
where
    T: NewService<
        Request = ServiceRequest<P>,
        Response = ServiceResponse<B>,
        Error = Error,
        InitError = (),
    >,
    C: NewService<
        Request = ServiceRequest,
        Response = ServiceRequest<P>,
        Error = Error,
        InitError = (),
    >,
{
    fn into_new_service(self) -> AppInit<C, T, P, B> {
        AppInit {
            chain: self.chain,
            data: self.data,
            endpoint: self.endpoint,
            services: RefCell::new(self.services),
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
    use futures::{Future, IntoFuture};

    use super::*;
    use crate::http::{header, HeaderValue, Method, StatusCode};
    use crate::service::{ServiceRequest, ServiceResponse};
    use crate::test::{block_on, call_success, init_service, TestRequest};
    use crate::{web, Error, HttpResponse};

    #[test]
    fn test_default_resource() {
        let mut srv = init_service(
            App::new().service(web::resource("/test").to(|| HttpResponse::Ok())),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = TestRequest::with_uri("/blah").to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = init_service(
            App::new()
                .service(web::resource("/test").to(|| HttpResponse::Ok()))
                .service(
                    web::resource("/test2")
                        .default_resource(|r| r.to(|| HttpResponse::Created()))
                        .route(web::get().to(|| HttpResponse::Ok())),
                )
                .default_resource(|r| r.to(|| HttpResponse::MethodNotAllowed())),
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

    fn md<S, P, B>(
        req: ServiceRequest<P>,
        srv: &mut S,
    ) -> impl IntoFuture<Item = ServiceResponse<B>, Error = Error>
    where
        S: Service<
            Request = ServiceRequest<P>,
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
        let resp = call_success(&mut srv, req);
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
        let resp = call_success(&mut srv, req);
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
        let resp = call_success(&mut srv, req);
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
        let resp = call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("0001")
        );
    }
}

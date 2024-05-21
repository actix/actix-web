use std::{net::SocketAddr, rc::Rc};

use actix_service::{boxed, IntoServiceFactory, ServiceFactory, ServiceFactoryExt as _};

use crate::{
    data::Data,
    dev::{Extensions, ResourceDef},
    error::Error,
    guard::Guard,
    resource::Resource,
    rmap::ResourceMap,
    route::Route,
    service::{
        AppServiceFactory, BoxedHttpServiceFactory, HttpServiceFactory, ServiceFactoryWrapper,
        ServiceRequest, ServiceResponse,
    },
};

type Guards = Vec<Box<dyn Guard>>;

/// Application configuration
pub struct AppService {
    config: AppConfig,
    root: bool,
    default: Rc<BoxedHttpServiceFactory>,
    #[allow(clippy::type_complexity)]
    services: Vec<(
        ResourceDef,
        BoxedHttpServiceFactory,
        Option<Guards>,
        Option<Rc<ResourceMap>>,
    )>,
}

impl AppService {
    /// Crate server settings instance.
    pub(crate) fn new(config: AppConfig, default: Rc<BoxedHttpServiceFactory>) -> Self {
        AppService {
            config,
            default,
            root: true,
            services: Vec::new(),
        }
    }

    /// Check if root is being configured
    pub fn is_root(&self) -> bool {
        self.root
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn into_services(
        self,
    ) -> (
        AppConfig,
        Vec<(
            ResourceDef,
            BoxedHttpServiceFactory,
            Option<Guards>,
            Option<Rc<ResourceMap>>,
        )>,
    ) {
        (self.config, self.services)
    }

    /// Clones inner config and default service, returning new `AppService` with empty service list
    /// marked as non-root.
    pub(crate) fn clone_config(&self) -> Self {
        AppService {
            config: self.config.clone(),
            default: self.default.clone(),
            services: Vec::new(),
            root: false,
        }
    }

    /// Returns reference to configuration.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Returns default handler factory.
    pub fn default_service(&self) -> Rc<BoxedHttpServiceFactory> {
        self.default.clone()
    }

    /// Register HTTP service.
    pub fn register_service<F, S>(
        &mut self,
        rdef: ResourceDef,
        guards: Option<Vec<Box<dyn Guard>>>,
        factory: F,
        nested: Option<Rc<ResourceMap>>,
    ) where
        F: IntoServiceFactory<S, ServiceRequest>,
        S: ServiceFactory<
                ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
                Config = (),
                InitError = (),
            > + 'static,
    {
        self.services
            .push((rdef, boxed::factory(factory.into_factory()), guards, nested));
    }
}

/// Application connection config.
#[derive(Debug, Clone)]
pub struct AppConfig {
    secure: bool,
    host: String,
    addr: SocketAddr,
}

impl AppConfig {
    pub(crate) fn new(secure: bool, host: String, addr: SocketAddr) -> Self {
        AppConfig { secure, host, addr }
    }

    /// Needed in actix-test crate. Semver exempt.
    #[doc(hidden)]
    pub fn __priv_test_new(secure: bool, host: String, addr: SocketAddr) -> Self {
        AppConfig::new(secure, host, addr)
    }

    /// Server host name.
    ///
    /// Host name is used by application router as a hostname for URL generation.
    /// Check [ConnectionInfo](super::dev::ConnectionInfo::host())
    /// documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns true if connection is secure (i.e., running over `https:`).
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns the socket address of the local half of this TCP connection.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    #[cfg(test)]
    pub(crate) fn set_host(&mut self, host: &str) {
        host.clone_into(&mut self.host);
    }
}

impl Default for AppConfig {
    /// Returns the default AppConfig.
    /// Note: The included socket address is "127.0.0.1".
    ///
    /// 127.0.0.1: non-routable meta address that denotes an unknown, invalid or non-applicable target.
    /// If you need a service only accessed by itself, use a loopback address.
    /// A loopback address for IPv4 is any loopback address that begins with "127".
    /// Loopback addresses should be only used to test your application locally.
    /// The default configuration provides a loopback address.
    ///
    /// 0.0.0.0: if configured to use this special address, the application will listen to any IP address configured on the machine.
    fn default() -> Self {
        AppConfig::new(
            false,
            "localhost:8080".to_owned(),
            "127.0.0.1:8080".parse().unwrap(),
        )
    }
}

/// Enables parts of app configuration to be declared separately from the app itself. Helpful for
/// modularizing large applications.
///
/// Merge a `ServiceConfig` into an app using [`App::configure`](crate::App::configure). Scope and
/// resources services have similar methods.
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
/// // merge `/test` routes from config function to App
/// App::new().configure(config);
/// ```
pub struct ServiceConfig {
    pub(crate) services: Vec<Box<dyn AppServiceFactory>>,
    pub(crate) external: Vec<ResourceDef>,
    pub(crate) app_data: Extensions,
    pub(crate) default: Option<Rc<BoxedHttpServiceFactory>>,
}

impl ServiceConfig {
    pub(crate) fn new() -> Self {
        Self {
            services: Vec::new(),
            external: Vec::new(),
            app_data: Extensions::new(),
            default: None,
        }
    }

    /// Add shared app data item.
    ///
    /// Counterpart to [`App::data()`](crate::App::data).
    #[deprecated(since = "4.0.0", note = "Use `.app_data(Data::new(val))` instead.")]
    pub fn data<U: 'static>(&mut self, data: U) -> &mut Self {
        self.app_data(Data::new(data));
        self
    }

    /// Add arbitrary app data item.
    ///
    /// Counterpart to [`App::app_data()`](crate::App::app_data).
    pub fn app_data<U: 'static>(&mut self, ext: U) -> &mut Self {
        self.app_data.insert(ext);
        self
    }

    /// Default service to be used if no matching resource could be found.
    ///
    /// Counterpart to [`App::default_service()`](crate::App::default_service).
    pub fn default_service<F, U>(&mut self, f: F) -> &mut Self
    where
        F: IntoServiceFactory<U, ServiceRequest>,
        U: ServiceFactory<ServiceRequest, Config = (), Response = ServiceResponse, Error = Error>
            + 'static,
        U::InitError: std::fmt::Debug,
    {
        let svc = f
            .into_factory()
            .map_init_err(|err| log::error!("Can not construct default service: {:?}", err));

        self.default = Some(Rc::new(boxed::factory(svc)));

        self
    }

    /// Run external configuration as part of the application building process
    ///
    /// Counterpart to [`App::configure()`](crate::App::configure) that allows for easy nesting.
    pub fn configure<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut ServiceConfig),
    {
        f(self);
        self
    }

    /// Configure route for a specific path.
    ///
    /// Counterpart to [`App::route()`](crate::App::route).
    pub fn route(&mut self, path: &str, mut route: Route) -> &mut Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register HTTP service factory.
    ///
    /// Counterpart to [`App::service()`](crate::App::service).
    pub fn service<F>(&mut self, factory: F) -> &mut Self
    where
        F: HttpServiceFactory + 'static,
    {
        self.services
            .push(Box::new(ServiceFactoryWrapper::new(factory)));
        self
    }

    /// Register an external resource.
    ///
    /// External resources are useful for URL generation purposes only and are never considered for
    /// matching at request time. Calls to [`HttpRequest::url_for()`](crate::HttpRequest::url_for)
    /// will work as expected.
    ///
    /// Counterpart to [`App::external_resource()`](crate::App::external_resource).
    pub fn external_resource<N, U>(&mut self, name: N, url: U) -> &mut Self
    where
        N: AsRef<str>,
        U: AsRef<str>,
    {
        let mut rdef = ResourceDef::new(url.as_ref());
        rdef.set_name(name.as_ref());
        self.external.push(rdef);
        self
    }
}

#[cfg(test)]
mod tests {
    use actix_service::Service;
    use bytes::Bytes;

    use super::*;
    use crate::{
        http::{Method, StatusCode},
        test::{assert_body_eq, call_service, init_service, read_body, TestRequest},
        web, App, HttpRequest, HttpResponse,
    };

    // allow deprecated `ServiceConfig::data`
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_data() {
        let cfg = |cfg: &mut ServiceConfig| {
            cfg.data(10usize);
            cfg.app_data(15u8);
        };

        let srv = init_service(App::new().configure(cfg).service(web::resource("/").to(
            |_: web::Data<usize>, req: HttpRequest| {
                assert_eq!(*req.app_data::<u8>().unwrap(), 15u8);
                HttpResponse::Ok()
            },
        )))
        .await;
        let req = TestRequest::default().to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_external_resource() {
        let srv = init_service(
            App::new()
                .configure(|cfg| {
                    cfg.external_resource("youtube", "https://youtube.com/watch/{video_id}");
                })
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

    #[actix_rt::test]
    async fn registers_default_service() {
        let srv = init_service(
            App::new()
                .configure(|cfg| {
                    cfg.default_service(
                        web::get().to(|| HttpResponse::NotFound().body("four oh four")),
                    );
                })
                .service(web::scope("/scoped").configure(|cfg| {
                    cfg.default_service(
                        web::get().to(|| HttpResponse::NotFound().body("scoped four oh four")),
                    );
                })),
        )
        .await;

        // app registers default service
        let req = TestRequest::with_uri("/path/i/did/not-configure").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"four oh four"));

        // scope registers default service
        let req = TestRequest::with_uri("/scoped/path/i/did/not-configure").to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = read_body(resp).await;
        assert_eq!(body, Bytes::from_static(b"scoped four oh four"));
    }

    #[actix_rt::test]
    async fn test_service() {
        let srv = init_service(App::new().configure(|cfg| {
            cfg.service(web::resource("/test").route(web::get().to(HttpResponse::Created)))
                .route("/index.html", web::get().to(HttpResponse::Ok));
        }))
        .await;

        let req = TestRequest::with_uri("/test")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/index.html")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&srv, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn nested_service_configure() {
        fn cfg_root(cfg: &mut ServiceConfig) {
            cfg.configure(cfg_sub);
        }

        fn cfg_sub(cfg: &mut ServiceConfig) {
            cfg.route("/", web::get().to(|| async { "hello world" }));
        }

        let srv = init_service(App::new().configure(cfg_root)).await;

        let req = TestRequest::with_uri("/").to_request();
        let res = call_service(&srv, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_body_eq!(res, b"hello world");
    }
}

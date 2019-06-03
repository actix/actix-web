use std::net::SocketAddr;
use std::rc::Rc;

use actix_http::Extensions;
use actix_router::ResourceDef;
use actix_service::{boxed, IntoNewService, NewService};

use crate::data::{Data, DataFactory};
use crate::error::Error;
use crate::guard::Guard;
use crate::resource::Resource;
use crate::rmap::ResourceMap;
use crate::route::Route;
use crate::service::{
    HttpServiceFactory, ServiceFactory, ServiceFactoryWrapper, ServiceRequest,
    ServiceResponse,
};

type Guards = Vec<Box<Guard>>;
type HttpNewService =
    boxed::BoxedNewService<(), ServiceRequest, ServiceResponse, Error, ()>;

/// Application configuration
pub struct AppService {
    config: AppConfig,
    root: bool,
    default: Rc<HttpNewService>,
    services: Vec<(
        ResourceDef,
        HttpNewService,
        Option<Guards>,
        Option<Rc<ResourceMap>>,
    )>,
    service_data: Rc<Vec<Box<DataFactory>>>,
}

impl AppService {
    /// Crate server settings instance
    pub(crate) fn new(
        config: AppConfig,
        default: Rc<HttpNewService>,
        service_data: Rc<Vec<Box<DataFactory>>>,
    ) -> Self {
        AppService {
            config,
            default,
            service_data,
            root: true,
            services: Vec::new(),
        }
    }

    /// Check if root is beeing configured
    pub fn is_root(&self) -> bool {
        self.root
    }

    pub(crate) fn into_services(
        self,
    ) -> (
        AppConfig,
        Vec<(
            ResourceDef,
            HttpNewService,
            Option<Guards>,
            Option<Rc<ResourceMap>>,
        )>,
    ) {
        (self.config, self.services)
    }

    pub(crate) fn clone_config(&self) -> Self {
        AppService {
            config: self.config.clone(),
            default: self.default.clone(),
            services: Vec::new(),
            root: false,
            service_data: self.service_data.clone(),
        }
    }

    /// Service configuration
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Default resource
    pub fn default_service(&self) -> Rc<HttpNewService> {
        self.default.clone()
    }

    /// Set global route data
    pub fn set_service_data(&self, extensions: &mut Extensions) -> bool {
        for f in self.service_data.iter() {
            f.create(extensions);
        }
        !self.service_data.is_empty()
    }

    /// Register http service
    pub fn register_service<F, S>(
        &mut self,
        rdef: ResourceDef,
        guards: Option<Vec<Box<Guard>>>,
        service: F,
        nested: Option<Rc<ResourceMap>>,
    ) where
        F: IntoNewService<S>,
        S: NewService<
                Config = (),
                Request = ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
                InitError = (),
            > + 'static,
    {
        self.services.push((
            rdef,
            boxed::new_service(service.into_new_service()),
            guards,
            nested,
        ));
    }
}

#[derive(Clone)]
pub struct AppConfig(pub(crate) Rc<AppConfigInner>);

impl AppConfig {
    pub(crate) fn new(inner: AppConfigInner) -> Self {
        AppConfig(Rc::new(inner))
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn host(&self) -> &str {
        &self.0.host
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.0.secure
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> SocketAddr {
        self.0.addr
    }
}

pub(crate) struct AppConfigInner {
    pub(crate) secure: bool,
    pub(crate) host: String,
    pub(crate) addr: SocketAddr,
}

impl Default for AppConfigInner {
    fn default() -> AppConfigInner {
        AppConfigInner {
            secure: false,
            addr: "127.0.0.1:8080".parse().unwrap(),
            host: "localhost:8080".to_owned(),
        }
    }
}

/// Service config is used for external configuration.
/// Part of application configuration could be offloaded
/// to set of external methods. This could help with
/// modularization of big application configuration.
pub struct ServiceConfig {
    pub(crate) services: Vec<Box<ServiceFactory>>,
    pub(crate) data: Vec<Box<DataFactory>>,
    pub(crate) external: Vec<ResourceDef>,
}

impl ServiceConfig {
    pub(crate) fn new() -> Self {
        Self {
            services: Vec::new(),
            data: Vec::new(),
            external: Vec::new(),
        }
    }

    /// Set application data. Application data could be accessed
    /// by using `Data<T>` extractor where `T` is data type.
    ///
    /// This is same as `App::data()` method.
    pub fn data<S: 'static>(&mut self, data: S) -> &mut Self {
        self.data.push(Box::new(Data::new(data)));
        self
    }

    /// Configure route for a specific path.
    ///
    /// This is same as `App::route()` method.
    pub fn route(&mut self, path: &str, mut route: Route) -> &mut Self {
        self.service(
            Resource::new(path)
                .add_guards(route.take_guards())
                .route(route),
        )
    }

    /// Register http service.
    ///
    /// This is same as `App::service()` method.
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
    /// External resources are useful for URL generation purposes only
    /// and are never considered for matching at request time. Calls to
    /// `HttpRequest::url_for()` will work as expected.
    ///
    /// This is same as `App::external_service()` method.
    pub fn external_resource<N, U>(&mut self, name: N, url: U) -> &mut Self
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

#[cfg(test)]
mod tests {
    use actix_service::Service;
    use bytes::Bytes;

    use super::*;
    use crate::http::{Method, StatusCode};
    use crate::test::{block_on, call_service, init_service, read_body, TestRequest};
    use crate::{web, App, HttpRequest, HttpResponse};

    #[test]
    fn test_data() {
        let cfg = |cfg: &mut ServiceConfig| {
            cfg.data(10usize);
        };

        let mut srv =
            init_service(App::new().configure(cfg).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // #[test]
    // fn test_data_factory() {
    //     let cfg = |cfg: &mut ServiceConfig| {
    //         cfg.data_factory(|| {
    //             sleep(std::time::Duration::from_millis(50)).then(|_| {
    //                 println!("READY");
    //                 Ok::<_, ()>(10usize)
    //             })
    //         });
    //     };

    //     let mut srv =
    //         init_service(App::new().configure(cfg).service(
    //             web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
    //         ));
    //     let req = TestRequest::default().to_request();
    //     let resp = block_on(srv.call(req)).unwrap();
    //     assert_eq!(resp.status(), StatusCode::OK);

    //     let cfg2 = |cfg: &mut ServiceConfig| {
    //         cfg.data_factory(|| Ok::<_, ()>(10u32));
    //     };
    //     let mut srv = init_service(
    //         App::new()
    //             .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()))
    //             .configure(cfg2),
    //     );
    //     let req = TestRequest::default().to_request();
    //     let resp = block_on(srv.call(req)).unwrap();
    //     assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    // }

    #[test]
    fn test_external_resource() {
        let mut srv = init_service(
            App::new()
                .configure(|cfg| {
                    cfg.external_resource(
                        "youtube",
                        "https://youtube.com/watch/{video_id}",
                    );
                })
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

    #[test]
    fn test_service() {
        let mut srv = init_service(App::new().configure(|cfg| {
            cfg.service(
                web::resource("/test").route(web::get().to(|| HttpResponse::Created())),
            )
            .route("/index.html", web::get().to(|| HttpResponse::Ok()));
        }));

        let req = TestRequest::with_uri("/test")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::CREATED);

        let req = TestRequest::with_uri("/index.html")
            .method(Method::GET)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

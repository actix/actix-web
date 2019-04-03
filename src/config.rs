use std::cell::{Ref, RefCell};
use std::net::SocketAddr;
use std::rc::Rc;

use actix_http::Extensions;
use actix_router::ResourceDef;
use actix_service::{boxed, IntoNewService, NewService};
use futures::IntoFuture;

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
type HttpNewService<P> =
    boxed::BoxedNewService<(), ServiceRequest<P>, ServiceResponse, Error, ()>;

/// Application configuration
pub struct ServiceConfig<P> {
    config: AppConfig,
    root: bool,
    default: Rc<HttpNewService<P>>,
    services: Vec<(
        ResourceDef,
        HttpNewService<P>,
        Option<Guards>,
        Option<Rc<ResourceMap>>,
    )>,
}

impl<P: 'static> ServiceConfig<P> {
    /// Crate server settings instance
    pub(crate) fn new(config: AppConfig, default: Rc<HttpNewService<P>>) -> Self {
        ServiceConfig {
            config,
            default,
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
    ) -> Vec<(
        ResourceDef,
        HttpNewService<P>,
        Option<Guards>,
        Option<Rc<ResourceMap>>,
    )> {
        self.services
    }

    pub(crate) fn clone_config(&self) -> Self {
        ServiceConfig {
            config: self.config.clone(),
            default: self.default.clone(),
            services: Vec::new(),
            root: false,
        }
    }

    /// Service configuration
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Default resource
    pub fn default_service(&self) -> Rc<HttpNewService<P>> {
        self.default.clone()
    }

    pub fn register_service<F, S>(
        &mut self,
        rdef: ResourceDef,
        guards: Option<Vec<Box<Guard>>>,
        service: F,
        nested: Option<Rc<ResourceMap>>,
    ) where
        F: IntoNewService<S>,
        S: NewService<
                Request = ServiceRequest<P>,
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

    /// Resource map
    pub fn rmap(&self) -> &ResourceMap {
        &self.0.rmap
    }

    /// Application extensions
    pub fn extensions(&self) -> Ref<Extensions> {
        self.0.extensions.borrow()
    }
}

pub(crate) struct AppConfigInner {
    pub(crate) secure: bool,
    pub(crate) host: String,
    pub(crate) addr: SocketAddr,
    pub(crate) rmap: ResourceMap,
    pub(crate) extensions: RefCell<Extensions>,
}

impl Default for AppConfigInner {
    fn default() -> AppConfigInner {
        AppConfigInner {
            secure: false,
            addr: "127.0.0.1:8080".parse().unwrap(),
            host: "localhost:8080".to_owned(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            extensions: RefCell::new(Extensions::new()),
        }
    }
}

/// Router config. It is used for external configuration.
/// Part of application configuration could be offloaded
/// to set of external methods. This could help with
/// modularization of big application configuration.
pub struct RouterConfig<P: 'static> {
    pub(crate) services: Vec<Box<ServiceFactory<P>>>,
    pub(crate) data: Vec<Box<DataFactory>>,
    pub(crate) external: Vec<ResourceDef>,
}

impl<P: 'static> RouterConfig<P> {
    pub(crate) fn new() -> Self {
        Self {
            services: Vec::new(),
            data: Vec::new(),
            external: Vec::new(),
        }
    }

    /// Set application data. Applicatin data could be accessed
    /// by using `Data<T>` extractor where `T` is data type.
    ///
    /// This is same as `App::data()` method.
    pub fn data<S: 'static>(&mut self, data: S) -> &mut Self {
        self.data.push(Box::new(Data::new(data)));
        self
    }

    /// Set application data factory. This function is
    /// similar to `.data()` but it accepts data factory. Data object get
    /// constructed asynchronously during application initialization.
    ///
    /// This is same as `App::data_dactory()` method.
    pub fn data_factory<F, R>(&mut self, data: F) -> &mut Self
    where
        F: Fn() -> R + 'static,
        R: IntoFuture + 'static,
        R::Error: std::fmt::Debug,
    {
        self.data.push(Box::new(data));
        self
    }

    /// Configure route for a specific path.
    ///
    /// This is same as `App::route()` method.
    pub fn route(&mut self, path: &str, mut route: Route<P>) -> &mut Self {
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
        F: HttpServiceFactory<P> + 'static,
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

    use super::*;
    use crate::http::StatusCode;
    use crate::test::{block_on, init_service, TestRequest};
    use crate::{web, App, HttpResponse};

    #[test]
    fn test_data() {
        let cfg = |cfg: &mut RouterConfig<_>| {
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

    #[test]
    fn test_data_factory() {
        let cfg = |cfg: &mut RouterConfig<_>| {
            cfg.data_factory(|| Ok::<_, ()>(10usize));
        };

        let mut srv =
            init_service(App::new().configure(cfg).service(
                web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()),
            ));
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let cfg2 = |cfg: &mut RouterConfig<_>| {
            cfg.data_factory(|| Ok::<_, ()>(10u32));
        };
        let mut srv = init_service(
            App::new()
                .service(web::resource("/").to(|_: web::Data<usize>| HttpResponse::Ok()))
                .configure(cfg2),
        );
        let req = TestRequest::default().to_request();
        let resp = block_on(srv.call(req)).unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}

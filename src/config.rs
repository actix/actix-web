use std::cell::{Ref, RefCell};
use std::net::SocketAddr;
use std::rc::Rc;

use actix_http::Extensions;
use actix_router::ResourceDef;
use actix_service::{boxed, IntoNewService, NewService};

use crate::error::Error;
use crate::guard::Guard;
use crate::rmap::ResourceMap;
use crate::service::{ServiceRequest, ServiceResponse};

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

use std::net::SocketAddr;
use std::rc::Rc;

use actix_router::ResourceDef;
use actix_service::{boxed, IntoNewService, NewService};

use crate::guard::Guard;
use crate::rmap::ResourceMap;
use crate::service::{ServiceRequest, ServiceResponse};

type Guards = Vec<Box<Guard>>;
type HttpNewService<P> =
    boxed::BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;

/// Application configuration
pub struct AppConfig<P> {
    addr: SocketAddr,
    secure: bool,
    host: String,
    root: bool,
    default: Rc<HttpNewService<P>>,
    services: Vec<(
        ResourceDef,
        HttpNewService<P>,
        Option<Guards>,
        Option<Rc<ResourceMap>>,
    )>,
}

impl<P: 'static> AppConfig<P> {
    /// Crate server settings instance
    pub(crate) fn new(
        addr: SocketAddr,
        host: String,
        secure: bool,
        default: Rc<HttpNewService<P>>,
    ) -> Self {
        AppConfig {
            addr,
            secure,
            host,
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
        AppConfig {
            addr: self.addr,
            secure: self.secure,
            host: self.host.clone(),
            default: self.default.clone(),
            services: Vec::new(),
            root: false,
        }
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns host header value
    pub fn host(&self) -> &str {
        &self.host
    }

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
                Error = (),
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

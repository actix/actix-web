use std::collections::HashMap;
use std::{fmt, io, net};

use futures::future::{join_all, Future};
use log::error;
use tokio_tcp::TcpStream;

use crate::counter::CounterGuard;
use crate::service::{IntoNewService, NewService};

use super::server::bind_addr;
use super::services::{
    BoxedServerService, InternalServiceFactory, ServerMessage, StreamService,
};
use super::Token;

pub struct ServiceConfig {
    pub(super) services: Vec<(String, net::TcpListener)>,
    pub(super) rt: Box<ServiceRuntimeConfiguration>,
}

impl ServiceConfig {
    pub(super) fn new() -> ServiceConfig {
        ServiceConfig {
            services: Vec::new(),
            rt: Box::new(not_configured),
        }
    }

    /// Add new service to server
    pub fn bind<U, N: AsRef<str>>(&mut self, name: N, addr: U) -> io::Result<&mut Self>
    where
        U: net::ToSocketAddrs,
    {
        let sockets = bind_addr(addr)?;

        for lst in sockets {
            self.listen(name.as_ref(), lst);
        }

        Ok(self)
    }

    /// Add new service to server
    pub fn listen<N: AsRef<str>>(&mut self, name: N, lst: net::TcpListener) -> &mut Self {
        self.services.push((name.as_ref().to_string(), lst));
        self
    }

    /// Register service configuration function
    pub fn rt<F>(&mut self, f: F) -> io::Result<()>
    where
        F: Fn(&mut ServiceRuntime) + Send + Clone + 'static,
    {
        self.rt = Box::new(f);
        Ok(())
    }
}

pub(super) struct ConfiguredService {
    rt: Box<ServiceRuntimeConfiguration>,
    names: HashMap<Token, String>,
    services: HashMap<String, Token>,
}

impl ConfiguredService {
    pub(super) fn new(rt: Box<ServiceRuntimeConfiguration>) -> Self {
        ConfiguredService {
            rt,
            names: HashMap::new(),
            services: HashMap::new(),
        }
    }

    pub(super) fn stream(&mut self, token: Token, name: String) {
        self.names.insert(token, name.clone());
        self.services.insert(name, token);
    }
}

impl InternalServiceFactory for ConfiguredService {
    fn name(&self, token: Token) -> &str {
        &self.names[&token]
    }

    fn clone_factory(&self) -> Box<InternalServiceFactory> {
        Box::new(Self {
            rt: self.rt.clone(),
            names: self.names.clone(),
            services: self.services.clone(),
        })
    }

    fn create(&self) -> Box<Future<Item = Vec<(Token, BoxedServerService)>, Error = ()>> {
        // configure services
        let mut rt = ServiceRuntime::new(self.services.clone());
        self.rt.configure(&mut rt);
        rt.validate();

        // construct services
        let mut fut = Vec::new();
        for (token, ns) in rt.services {
            fut.push(ns.new_service().map(move |service| (token, service)));
        }

        Box::new(join_all(fut).map_err(|e| {
            error!("Can not construct service: {:?}", e);
        }))
    }
}

pub(super) trait ServiceRuntimeConfiguration: Send {
    fn clone(&self) -> Box<ServiceRuntimeConfiguration>;

    fn configure(&self, rt: &mut ServiceRuntime);
}

impl<F> ServiceRuntimeConfiguration for F
where
    F: Fn(&mut ServiceRuntime) + Send + Clone + 'static,
{
    fn clone(&self) -> Box<ServiceRuntimeConfiguration> {
        Box::new(self.clone())
    }

    fn configure(&self, rt: &mut ServiceRuntime) {
        (self)(rt)
    }
}

fn not_configured(_: &mut ServiceRuntime) {
    error!("Service is not configured");
}

pub struct ServiceRuntime {
    names: HashMap<String, Token>,
    services: HashMap<Token, BoxedNewService>,
}

impl ServiceRuntime {
    fn new(names: HashMap<String, Token>) -> Self {
        ServiceRuntime {
            names,
            services: HashMap::new(),
        }
    }

    fn validate(&self) {
        for (name, token) in &self.names {
            if !self.services.contains_key(&token) {
                error!("Service {:?} is not configured", name);
            }
        }
    }

    pub fn service<T, F>(&mut self, name: &str, service: F)
    where
        F: IntoNewService<T, TcpStream>,
        T: NewService<TcpStream, Response = ()> + 'static,
        T::Future: 'static,
        T::Service: 'static,
        T::InitError: fmt::Debug,
    {
        // let name = name.to_owned();
        if let Some(token) = self.names.get(name) {
            self.services.insert(
                token.clone(),
                Box::new(ServiceFactory {
                    inner: service.into_new_service(),
                }),
            );
        } else {
            panic!("Unknown service: {:?}", name);
        }
    }
}

type BoxedNewService = Box<
    NewService<
        (Option<CounterGuard>, ServerMessage),
        Response = (),
        Error = (),
        InitError = (),
        Service = BoxedServerService,
        Future = Box<Future<Item = BoxedServerService, Error = ()>>,
    >,
>;

struct ServiceFactory<T> {
    inner: T,
}

impl<T> NewService<(Option<CounterGuard>, ServerMessage)> for ServiceFactory<T>
where
    T: NewService<TcpStream, Response = ()>,
    T::Future: 'static,
    T::Service: 'static,
    T::Error: 'static,
    T::InitError: fmt::Debug + 'static,
{
    type Response = ();
    type Error = ();
    type InitError = ();
    type Service = BoxedServerService;
    type Future = Box<Future<Item = BoxedServerService, Error = ()>>;

    fn new_service(&self) -> Self::Future {
        Box::new(self.inner.new_service().map_err(|_| ()).map(|s| {
            let service: BoxedServerService = Box::new(StreamService::new(s));
            service
        }))
    }
}

use std::{fmt, net};

use actix_net::either::Either;
use actix_net::server::{Server, ServiceFactory};
use actix_net::service::{NewService, NewServiceExt};

use super::acceptor::{
    AcceptorServiceFactory, AcceptorTimeout, ServerMessageAcceptor, TcpAcceptor,
};
use super::error::AcceptorError;
use super::handler::IntoHttpHandler;
use super::service::HttpService;
use super::settings::{ServerSettings, WorkerSettings};
use super::KeepAlive;

pub(crate) trait ServiceProvider {
    fn register(
        &self, server: Server, lst: net::TcpListener, host: String,
        addr: net::SocketAddr, keep_alive: KeepAlive, client_timeout: usize,
    ) -> Server;
}

/// Utility type that builds complete http pipeline
pub struct HttpServiceBuilder<F, H, A>
where
    F: Fn() -> H + Send + Clone,
{
    factory: F,
    acceptor: A,
    no_client_timer: bool,
}

impl<F, H, A> HttpServiceBuilder<F, H, A>
where
    F: Fn() -> H + Send + Clone + 'static,
    H: IntoHttpHandler,
    A: AcceptorServiceFactory,
    <A::NewService as NewService>::InitError: fmt::Debug,
{
    /// Create http service builder
    pub fn new(factory: F, acceptor: A) -> Self {
        Self {
            factory,
            acceptor,
            no_client_timer: false,
        }
    }

    pub(crate) fn no_client_timer(mut self) -> Self {
        self.no_client_timer = true;
        self
    }

    /// Use different acceptor factory
    pub fn acceptor<A1>(self, acceptor: A1) -> HttpServiceBuilder<F, H, A1>
    where
        A1: AcceptorServiceFactory,
        <A1::NewService as NewService>::InitError: fmt::Debug,
    {
        HttpServiceBuilder {
            acceptor,
            factory: self.factory.clone(),
            no_client_timer: self.no_client_timer,
        }
    }

    fn finish(
        &self, host: String, addr: net::SocketAddr, keep_alive: KeepAlive,
        client_timeout: usize,
    ) -> impl ServiceFactory {
        let timeout = if self.no_client_timer {
            0
        } else {
            client_timeout
        };
        let factory = self.factory.clone();
        let acceptor = self.acceptor.clone();
        move || {
            let app = (factory)().into_handler();
            let settings = WorkerSettings::new(
                app,
                keep_alive,
                timeout as u64,
                ServerSettings::new(addr, &host, false),
            );

            if timeout == 0 {
                Either::A(ServerMessageAcceptor::new(
                    settings.clone(),
                    TcpAcceptor::new(acceptor.create().map_err(AcceptorError::Service))
                        .map_err(|_| ())
                        .map_init_err(|_| ())
                        .and_then(
                            HttpService::new(settings)
                                .map_init_err(|_| ())
                                .map_err(|_| ()),
                        ),
                ))
            } else {
                Either::B(ServerMessageAcceptor::new(
                    settings.clone(),
                    TcpAcceptor::new(AcceptorTimeout::new(timeout, acceptor.create()))
                        .map_err(|_| ())
                        .map_init_err(|_| ())
                        .and_then(
                            HttpService::new(settings)
                                .map_init_err(|_| ())
                                .map_err(|_| ()),
                        ),
                ))
            }
        }
    }
}

impl<F, H, A> Clone for HttpServiceBuilder<F, H, A>
where
    F: Fn() -> H + Send + Clone,
    H: IntoHttpHandler,
    A: AcceptorServiceFactory,
{
    fn clone(&self) -> Self {
        HttpServiceBuilder {
            factory: self.factory.clone(),
            acceptor: self.acceptor.clone(),
            no_client_timer: self.no_client_timer,
        }
    }
}

impl<F, H, A> ServiceProvider for HttpServiceBuilder<F, H, A>
where
    F: Fn() -> H + Send + Clone + 'static,
    A: AcceptorServiceFactory,
    <A::NewService as NewService>::InitError: fmt::Debug,
    H: IntoHttpHandler,
{
    fn register(
        &self, server: Server, lst: net::TcpListener, host: String,
        addr: net::SocketAddr, keep_alive: KeepAlive, client_timeout: usize,
    ) -> Server {
        server.listen2(
            "actix-web",
            lst,
            self.finish(host, addr, keep_alive, client_timeout),
        )
    }
}

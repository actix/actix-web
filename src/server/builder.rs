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
use super::settings::{ServerSettings, ServiceConfig};
use super::KeepAlive;

pub(crate) trait ServiceProvider {
    fn register(
        &self, server: Server, lst: net::TcpListener, host: String,
        addr: net::SocketAddr, keep_alive: KeepAlive, secure: bool, client_timeout: u64,
        client_shutdown: u64,
    ) -> Server;
}

/// Utility type that builds complete http pipeline
pub(crate) struct HttpServiceBuilder<F, H, A>
where
    F: Fn() -> H + Send + Clone,
{
    factory: F,
    acceptor: A,
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
        Self { factory, acceptor }
    }

    fn finish(
        &self, host: String, addr: net::SocketAddr, keep_alive: KeepAlive, secure: bool,
        client_timeout: u64, client_shutdown: u64,
    ) -> impl ServiceFactory {
        let factory = self.factory.clone();
        let acceptor = self.acceptor.clone();
        move || {
            let app = (factory)().into_handler();
            let settings = ServiceConfig::new(
                app,
                keep_alive,
                client_timeout,
                client_shutdown,
                ServerSettings::new(addr, &host, false),
            );

            if secure {
                Either::B(ServerMessageAcceptor::new(
                    settings.clone(),
                    TcpAcceptor::new(AcceptorTimeout::new(
                        client_timeout,
                        acceptor.create(),
                    )).map_err(|_| ())
                    .map_init_err(|_| ())
                    .and_then(
                        HttpService::new(settings)
                            .map_init_err(|_| ())
                            .map_err(|_| ()),
                    ),
                ))
            } else {
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
            }
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
        addr: net::SocketAddr, keep_alive: KeepAlive, secure: bool, client_timeout: u64,
        client_shutdown: u64,
    ) -> Server {
        server.listen2(
            "actix-web",
            lst,
            self.finish(
                host,
                addr,
                keep_alive,
                secure,
                client_timeout,
                client_shutdown,
            ),
        )
    }
}

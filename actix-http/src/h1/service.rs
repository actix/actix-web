use std::marker::PhantomData;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::{fmt, net};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_rt::net::TcpStream;
use actix_service::{
    fn_service, IntoServiceFactory, Service, ServiceFactory, ServiceFactoryExt as _,
};
use actix_utils::future::ready;
use futures_core::future::LocalBoxFuture;

use crate::body::MessageBody;
use crate::config::ServiceConfig;
use crate::error::{DispatchError, Error};
use crate::request::Request;
use crate::response::Response;
use crate::service::HttpServiceHandler;
use crate::{ConnectCallback, OnConnectData};

use super::codec::Codec;
use super::dispatcher::Dispatcher;
use super::{ExpectHandler, UpgradeHandler};

/// `ServiceFactory` implementation for HTTP1 transport
pub struct H1Service<T, S, B, X = ExpectHandler, U = UpgradeHandler> {
    srv: S,
    cfg: ServiceConfig,
    expect: X,
    upgrade: Option<U>,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _phantom: PhantomData<B>,
}

impl<T, S, B> H1Service<T, S, B>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    /// Create new `HttpService` instance with config.
    pub(crate) fn with_config<F: IntoServiceFactory<S, Request>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        H1Service {
            cfg,
            srv: service.into_factory(),
            expect: ExpectHandler,
            upgrade: None,
            on_connect_ext: None,
            _phantom: PhantomData,
        }
    }
}

impl<S, B, X, U> H1Service<TcpStream, S, B, X, U>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    U: ServiceFactory<(Request, Framed<TcpStream, Codec>), Config = (), Response = ()>,
    U::Future: 'static,
    U::Error: fmt::Display + Into<Error>,
    U::InitError: fmt::Debug,
{
    /// Create simple tcp stream service
    pub fn tcp(
        self,
    ) -> impl ServiceFactory<
        TcpStream,
        Config = (),
        Response = (),
        Error = DispatchError,
        InitError = (),
    > {
        fn_service(|io: TcpStream| {
            let peer_addr = io.peer_addr().ok();
            ready(Ok((io, peer_addr)))
        })
        .and_then(self)
    }
}

#[cfg(feature = "openssl")]
mod openssl {
    use super::*;

    use actix_service::ServiceFactoryExt;
    use actix_tls::accept::{
        openssl::{Acceptor, SslAcceptor, SslError, TlsStream},
        TlsError,
    };

    impl<S, B, X, U> H1Service<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Error>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
        B: MessageBody,
        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Error>,
        X::InitError: fmt::Debug,
        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Error>,
        U::InitError: fmt::Debug,
    {
        /// Create openssl based service
        pub fn openssl(
            self,
            acceptor: SslAcceptor,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<SslError, DispatchError>,
            InitError = (),
        > {
            Acceptor::new(acceptor)
                .map_err(TlsError::Tls)
                .map_init_err(|_| panic!())
                .and_then(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().peer_addr().ok();
                    ready(Ok((io, peer_addr)))
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls")]
mod rustls {
    use super::*;

    use std::io;

    use actix_service::ServiceFactoryExt;
    use actix_tls::accept::{
        rustls::{Acceptor, ServerConfig, TlsStream},
        TlsError,
    };

    impl<S, B, X, U> H1Service<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Error>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,
        B: MessageBody,
        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Error>,
        X::InitError: fmt::Debug,
        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Error>,
        U::InitError: fmt::Debug,
    {
        /// Create rustls based service
        pub fn rustls(
            self,
            config: ServerConfig,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = (),
        > {
            Acceptor::new(config)
                .map_err(TlsError::Tls)
                .map_init_err(|_| panic!())
                .and_then(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().0.peer_addr().ok();
                    ready(Ok((io, peer_addr)))
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

impl<T, S, B, X, U> H1Service<T, S, B, X, U>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
{
    pub fn expect<X1>(self, expect: X1) -> H1Service<T, S, B, X1, U>
    where
        X1: ServiceFactory<Request, Response = Request>,
        X1::Error: Into<Error>,
        X1::InitError: fmt::Debug,
    {
        H1Service {
            expect,
            cfg: self.cfg,
            srv: self.srv,
            upgrade: self.upgrade,
            on_connect_ext: self.on_connect_ext,
            _phantom: PhantomData,
        }
    }

    pub fn upgrade<U1>(self, upgrade: Option<U1>) -> H1Service<T, S, B, X, U1>
    where
        U1: ServiceFactory<(Request, Framed<T, Codec>), Response = ()>,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
    {
        H1Service {
            upgrade,
            cfg: self.cfg,
            srv: self.srv,
            expect: self.expect,
            on_connect_ext: self.on_connect_ext,
            _phantom: PhantomData,
        }
    }

    /// Set on connect callback.
    pub(crate) fn on_connect_ext(mut self, f: Option<Rc<ConnectCallback<T>>>) -> Self {
        self.on_connect_ext = f;
        self
    }
}

impl<T, S, B, X, U> ServiceFactory<(T, Option<net::SocketAddr>)>
    for H1Service<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    U: ServiceFactory<(Request, Framed<T, Codec>), Config = (), Response = ()>,
    U::Future: 'static,
    U::Error: fmt::Display + Into<Error>,
    U::InitError: fmt::Debug,
{
    type Response = ();
    type Error = DispatchError;
    type Config = ();
    type Service = H1ServiceHandler<T, S::Service, B, X::Service, U::Service>;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = self.srv.new_service(());
        let expect = self.expect.new_service(());
        let upgrade = self.upgrade.as_ref().map(|s| s.new_service(()));
        let on_connect_ext = self.on_connect_ext.clone();
        let cfg = self.cfg.clone();

        Box::pin(async move {
            let expect = expect
                .await
                .map_err(|e| log::error!("Init http expect service error: {:?}", e))?;

            let upgrade = match upgrade {
                Some(upgrade) => {
                    let upgrade = upgrade.await.map_err(|e| {
                        log::error!("Init http upgrade service error: {:?}", e)
                    })?;
                    Some(upgrade)
                }
                None => None,
            };

            let service = service
                .await
                .map_err(|e| log::error!("Init http service error: {:?}", e))?;

            Ok(H1ServiceHandler::new(
                cfg,
                service,
                expect,
                upgrade,
                on_connect_ext,
            ))
        })
    }
}

/// `Service` implementation for HTTP/1 transport
pub type H1ServiceHandler<T, S, B, X, U> = HttpServiceHandler<T, S, B, X, U>;

impl<T, S, B, X, U> Service<(T, Option<net::SocketAddr>)>
    for HttpServiceHandler<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<(Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display + Into<Error>,
{
    type Response = ();
    type Error = DispatchError;
    type Future = Dispatcher<T, S, B, X, U>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self._poll_ready(cx).map_err(|e| {
            log::error!("HTTP/1 service readiness error: {:?}", e);
            DispatchError::Service(e)
        })
    }

    fn call(&self, (io, addr): (T, Option<net::SocketAddr>)) -> Self::Future {
        let on_connect_data =
            OnConnectData::from_io(&io, self.on_connect_ext.as_deref());

        Dispatcher::new(
            io,
            self.cfg.clone(),
            self.flow.clone(),
            on_connect_data,
            addr,
        )
    }
}

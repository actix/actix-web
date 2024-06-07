use std::{
    fmt,
    marker::PhantomData,
    net,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_rt::net::TcpStream;
use actix_service::{
    fn_service, IntoServiceFactory, Service, ServiceFactory, ServiceFactoryExt as _,
};
use actix_utils::future::ready;
use futures_core::future::LocalBoxFuture;
use tracing::error;

use super::{codec::Codec, dispatcher::Dispatcher, ExpectHandler, UpgradeHandler};
use crate::{
    body::{BoxBody, MessageBody},
    config::ServiceConfig,
    error::DispatchError,
    service::HttpServiceHandler,
    ConnectCallback, OnConnectData, Request, Response,
};

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
    S::Error: Into<Response<BoxBody>>,
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
    S::Error: Into<Response<BoxBody>>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,

    B: MessageBody,

    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Response<BoxBody>>,
    X::InitError: fmt::Debug,

    U: ServiceFactory<(Request, Framed<TcpStream, Codec>), Config = (), Response = ()>,
    U::Future: 'static,
    U::Error: fmt::Display + Into<Response<BoxBody>>,
    U::InitError: fmt::Debug,
{
    /// Create simple tcp stream service
    pub fn tcp(
        self,
    ) -> impl ServiceFactory<TcpStream, Config = (), Response = (), Error = DispatchError, InitError = ()>
    {
        fn_service(|io: TcpStream| {
            let peer_addr = io.peer_addr().ok();
            ready(Ok((io, peer_addr)))
        })
        .and_then(self)
    }
}

#[cfg(feature = "openssl")]
mod openssl {
    use actix_tls::accept::{
        openssl::{
            reexports::{Error as SslError, SslAcceptor},
            Acceptor, TlsStream,
        },
        TlsError,
    };

    use super::*;

    impl<S, B, X, U> H1Service<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,

        B: MessageBody,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Response<BoxBody>>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Response<BoxBody>>,
        U::InitError: fmt::Debug,
    {
        /// Create OpenSSL based service.
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
                .map_init_err(|_| {
                    unreachable!("TLS acceptor service factory does not error on init")
                })
                .map_err(TlsError::into_service_error)
                .map(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().peer_addr().ok();
                    (io, peer_addr)
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls-0_20")]
mod rustls_0_20 {
    use std::io;

    use actix_service::ServiceFactoryExt as _;
    use actix_tls::accept::{
        rustls_0_20::{reexports::ServerConfig, Acceptor, TlsStream},
        TlsError,
    };

    use super::*;

    impl<S, B, X, U> H1Service<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,

        B: MessageBody,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Response<BoxBody>>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Response<BoxBody>>,
        U::InitError: fmt::Debug,
    {
        /// Create Rustls v0.20 based service.
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
                .map_init_err(|_| {
                    unreachable!("TLS acceptor service factory does not error on init")
                })
                .map_err(TlsError::into_service_error)
                .map(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().0.peer_addr().ok();
                    (io, peer_addr)
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls-0_21")]
mod rustls_0_21 {
    use std::io;

    use actix_service::ServiceFactoryExt as _;
    use actix_tls::accept::{
        rustls_0_21::{reexports::ServerConfig, Acceptor, TlsStream},
        TlsError,
    };

    use super::*;

    impl<S, B, X, U> H1Service<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,

        B: MessageBody,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Response<BoxBody>>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Response<BoxBody>>,
        U::InitError: fmt::Debug,
    {
        /// Create Rustls v0.21 based service.
        pub fn rustls_021(
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
                .map_init_err(|_| {
                    unreachable!("TLS acceptor service factory does not error on init")
                })
                .map_err(TlsError::into_service_error)
                .map(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().0.peer_addr().ok();
                    (io, peer_addr)
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls-0_22")]
mod rustls_0_22 {
    use std::io;

    use actix_service::ServiceFactoryExt as _;
    use actix_tls::accept::{
        rustls_0_22::{reexports::ServerConfig, Acceptor, TlsStream},
        TlsError,
    };

    use super::*;

    impl<S, B, X, U> H1Service<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,

        B: MessageBody,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Response<BoxBody>>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Response<BoxBody>>,
        U::InitError: fmt::Debug,
    {
        /// Create Rustls v0.22 based service.
        pub fn rustls_0_22(
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
                .map_init_err(|_| {
                    unreachable!("TLS acceptor service factory does not error on init")
                })
                .map_err(TlsError::into_service_error)
                .map(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().0.peer_addr().ok();
                    (io, peer_addr)
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls-0_23")]
mod rustls_0_23 {
    use std::io;

    use actix_service::ServiceFactoryExt as _;
    use actix_tls::accept::{
        rustls_0_23::{reexports::ServerConfig, Acceptor, TlsStream},
        TlsError,
    };

    use super::*;

    impl<S, B, X, U> H1Service<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>>,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>>,

        B: MessageBody,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Response<BoxBody>>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Response<BoxBody>>,
        U::InitError: fmt::Debug,
    {
        /// Create Rustls v0.23 based service.
        pub fn rustls_0_23(
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
                .map_init_err(|_| {
                    unreachable!("TLS acceptor service factory does not error on init")
                })
                .map_err(TlsError::into_service_error)
                .map(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().0.peer_addr().ok();
                    (io, peer_addr)
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

impl<T, S, B, X, U> H1Service<T, S, B, X, U>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Response<BoxBody>>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
{
    pub fn expect<X1>(self, expect: X1) -> H1Service<T, S, B, X1, U>
    where
        X1: ServiceFactory<Request, Response = Request>,
        X1::Error: Into<Response<BoxBody>>,
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

impl<T, S, B, X, U> ServiceFactory<(T, Option<net::SocketAddr>)> for H1Service<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,

    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Response<BoxBody>>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,

    B: MessageBody,

    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Response<BoxBody>>,
    X::InitError: fmt::Debug,

    U: ServiceFactory<(Request, Framed<T, Codec>), Config = (), Response = ()>,
    U::Future: 'static,
    U::Error: fmt::Display + Into<Response<BoxBody>>,
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
                .map_err(|e| error!("Init http expect service error: {:?}", e))?;

            let upgrade = match upgrade {
                Some(upgrade) => {
                    let upgrade = upgrade
                        .await
                        .map_err(|e| error!("Init http upgrade service error: {:?}", e))?;
                    Some(upgrade)
                }
                None => None,
            };

            let service = service
                .await
                .map_err(|e| error!("Init http service error: {:?}", e))?;

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

impl<T, S, B, X, U> Service<(T, Option<net::SocketAddr>)> for HttpServiceHandler<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,

    S: Service<Request>,
    S::Error: Into<Response<BoxBody>>,
    S::Response: Into<Response<B>>,

    B: MessageBody,

    X: Service<Request, Response = Request>,
    X::Error: Into<Response<BoxBody>>,

    U: Service<(Request, Framed<T, Codec>), Response = ()>,
    U::Error: fmt::Display + Into<Response<BoxBody>>,
{
    type Response = ();
    type Error = DispatchError;
    type Future = Dispatcher<T, S, B, X, U>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self._poll_ready(cx).map_err(|err| {
            error!("HTTP/1 service readiness error: {:?}", err);
            DispatchError::Service(err)
        })
    }

    fn call(&self, (io, addr): (T, Option<net::SocketAddr>)) -> Self::Future {
        let conn_data = OnConnectData::from_io(&io, self.on_connect_ext.as_deref());
        Dispatcher::new(io, self.flow.clone(), self.cfg.clone(), addr, conn_data)
    }
}

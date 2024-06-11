use std::{
    future::Future,
    marker::PhantomData,
    mem, net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::net::TcpStream;
use actix_service::{
    fn_factory, fn_service, IntoServiceFactory, Service, ServiceFactory, ServiceFactoryExt as _,
};
use actix_utils::future::ready;
use futures_core::{future::LocalBoxFuture, ready};
use tracing::{error, trace};

use super::{dispatcher::Dispatcher, handshake_with_timeout, HandshakeWithTimeout};
use crate::{
    body::{BoxBody, MessageBody},
    config::ServiceConfig,
    error::DispatchError,
    service::HttpFlow,
    ConnectCallback, OnConnectData, Request, Response,
};

/// `ServiceFactory` implementation for HTTP/2 transport
pub struct H2Service<T, S, B> {
    srv: S,
    cfg: ServiceConfig,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _phantom: PhantomData<(T, B)>,
}

impl<T, S, B> H2Service<T, S, B>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,

    B: MessageBody + 'static,
{
    /// Create new `H2Service` instance with config.
    pub(crate) fn with_config<F: IntoServiceFactory<S, Request>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        H2Service {
            cfg,
            on_connect_ext: None,
            srv: service.into_factory(),
            _phantom: PhantomData,
        }
    }

    /// Set on connect callback.
    pub(crate) fn on_connect_ext(mut self, f: Option<Rc<ConnectCallback<T>>>) -> Self {
        self.on_connect_ext = f;
        self
    }
}

impl<S, B> H2Service<TcpStream, S, B>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,

    B: MessageBody + 'static,
{
    /// Create plain TCP based service
    pub fn tcp(
        self,
    ) -> impl ServiceFactory<
        TcpStream,
        Config = (),
        Response = (),
        Error = DispatchError,
        InitError = S::InitError,
    > {
        fn_factory(|| {
            ready(Ok::<_, S::InitError>(fn_service(|io: TcpStream| {
                let peer_addr = io.peer_addr().ok();
                ready(Ok::<_, DispatchError>((io, peer_addr)))
            })))
        })
        .and_then(self)
    }
}

#[cfg(feature = "openssl")]
mod openssl {
    use actix_service::ServiceFactoryExt as _;
    use actix_tls::accept::{
        openssl::{
            reexports::{Error as SslError, SslAcceptor},
            Acceptor, TlsStream,
        },
        TlsError,
    };

    use super::*;

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,
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
            InitError = S::InitError,
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
        rustls::{reexports::ServerConfig, Acceptor, TlsStream},
        TlsError,
    };

    use super::*;

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,
    {
        /// Create Rustls v0.20 based service.
        pub fn rustls(
            self,
            mut config: ServerConfig,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = S::InitError,
        > {
            let mut protos = vec![b"h2".to_vec()];
            protos.extend_from_slice(&config.alpn_protocols);
            config.alpn_protocols = protos;

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

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,
    {
        /// Create Rustls v0.21 based service.
        pub fn rustls_021(
            self,
            mut config: ServerConfig,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = S::InitError,
        > {
            let mut protos = vec![b"h2".to_vec()];
            protos.extend_from_slice(&config.alpn_protocols);
            config.alpn_protocols = protos;

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

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,
    {
        /// Create Rustls v0.22 based service.
        pub fn rustls_0_22(
            self,
            mut config: ServerConfig,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = S::InitError,
        > {
            let mut protos = vec![b"h2".to_vec()];
            protos.extend_from_slice(&config.alpn_protocols);
            config.alpn_protocols = protos;

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

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,
    {
        /// Create Rustls v0.23 based service.
        pub fn rustls_0_23(
            self,
            mut config: ServerConfig,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = S::InitError,
        > {
            let mut protos = vec![b"h2".to_vec()];
            protos.extend_from_slice(&config.alpn_protocols);
            config.alpn_protocols = protos;

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

impl<T, S, B> ServiceFactory<(T, Option<net::SocketAddr>)> for H2Service<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,

    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,

    B: MessageBody + 'static,
{
    type Response = ();
    type Error = DispatchError;
    type Config = ();
    type Service = H2ServiceHandler<T, S::Service, B>;
    type InitError = S::InitError;
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = self.srv.new_service(());
        let cfg = self.cfg.clone();
        let on_connect_ext = self.on_connect_ext.clone();

        Box::pin(async move {
            let service = service.await?;
            Ok(H2ServiceHandler::new(cfg, on_connect_ext, service))
        })
    }
}

/// `Service` implementation for HTTP/2 transport
pub struct H2ServiceHandler<T, S, B>
where
    S: Service<Request>,
{
    flow: Rc<HttpFlow<S, (), ()>>,
    cfg: ServiceConfig,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _phantom: PhantomData<B>,
}

impl<T, S, B> H2ServiceHandler<T, S, B>
where
    S: Service<Request>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    fn new(
        cfg: ServiceConfig,
        on_connect_ext: Option<Rc<ConnectCallback<T>>>,
        service: S,
    ) -> H2ServiceHandler<T, S, B> {
        H2ServiceHandler {
            flow: HttpFlow::new(service, (), None),
            cfg,
            on_connect_ext,
            _phantom: PhantomData,
        }
    }
}

impl<T, S, B> Service<(T, Option<net::SocketAddr>)> for H2ServiceHandler<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    type Response = ();
    type Error = DispatchError;
    type Future = H2ServiceHandlerResponse<T, S, B>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.flow.service.poll_ready(cx).map_err(|err| {
            let err = err.into();
            error!("Service readiness error: {:?}", err);
            DispatchError::Service(err)
        })
    }

    fn call(&self, (io, addr): (T, Option<net::SocketAddr>)) -> Self::Future {
        let on_connect_data = OnConnectData::from_io(&io, self.on_connect_ext.as_deref());

        H2ServiceHandlerResponse {
            state: State::Handshake(
                Some(self.flow.clone()),
                Some(self.cfg.clone()),
                addr,
                on_connect_data,
                handshake_with_timeout(io, &self.cfg),
            ),
        }
    }
}

enum State<T, S: Service<Request>, B: MessageBody>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S::Future: 'static,
{
    Handshake(
        Option<Rc<HttpFlow<S, (), ()>>>,
        Option<ServiceConfig>,
        Option<net::SocketAddr>,
        OnConnectData,
        HandshakeWithTimeout<T>,
    ),
    Established(Dispatcher<T, S, B, (), ()>),
}

pub struct H2ServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    state: State<T, S, B>,
}

impl<T, S, B> Future for H2ServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody,
{
    type Output = Result<(), DispatchError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state {
            State::Handshake(
                ref mut srv,
                ref mut config,
                ref peer_addr,
                ref mut conn_data,
                ref mut handshake,
            ) => match ready!(Pin::new(handshake).poll(cx)) {
                Ok((conn, timer)) => {
                    let on_connect_data = mem::take(conn_data);

                    self.state = State::Established(Dispatcher::new(
                        conn,
                        srv.take().unwrap(),
                        config.take().unwrap(),
                        *peer_addr,
                        on_connect_data,
                        timer,
                    ));

                    self.poll(cx)
                }

                Err(err) => {
                    trace!("H2 handshake error: {}", err);
                    Poll::Ready(Err(err))
                }
            },

            State::Established(ref mut disp) => Pin::new(disp).poll(cx),
        }
    }
}

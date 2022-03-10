use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    net,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_rt::net::TcpStream;
use actix_service::{
    fn_service, IntoServiceFactory, Service, ServiceFactory, ServiceFactoryExt as _,
};
use futures_core::{future::LocalBoxFuture, ready};
use pin_project_lite::pin_project;

use crate::{
    body::{BoxBody, MessageBody},
    builder::HttpServiceBuilder,
    error::DispatchError,
    h1, ConnectCallback, OnConnectData, Protocol, Request, Response, ServiceConfig,
};

/// A `ServiceFactory` for HTTP/1.1 or HTTP/2 protocol.
pub struct HttpService<T, S, B, X = h1::ExpectHandler, U = h1::UpgradeHandler> {
    srv: S,
    cfg: ServiceConfig,
    expect: X,
    upgrade: Option<U>,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _phantom: PhantomData<B>,
}

impl<T, S, B> HttpService<T, S, B>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Constructs builder for `HttpService` instance.
    pub fn build() -> HttpServiceBuilder<T, S> {
        HttpServiceBuilder::default()
    }
}

impl<T, S, B> HttpService<T, S, B>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Constructs new `HttpService` instance from service with default config.
    pub fn new<F: IntoServiceFactory<S, Request>>(service: F) -> Self {
        HttpService {
            cfg: ServiceConfig::default(),
            srv: service.into_factory(),
            expect: h1::ExpectHandler,
            upgrade: None,
            on_connect_ext: None,
            _phantom: PhantomData,
        }
    }

    /// Constructs new `HttpService` instance from config and service.
    pub(crate) fn with_config<F: IntoServiceFactory<S, Request>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        HttpService {
            cfg,
            srv: service.into_factory(),
            expect: h1::ExpectHandler,
            upgrade: None,
            on_connect_ext: None,
            _phantom: PhantomData,
        }
    }
}

impl<T, S, B, X, U> HttpService<T, S, B, X, U>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody,
{
    /// Sets service for `Expect: 100-Continue` handling.
    ///
    /// An expect service is called with requests that contain an `Expect` header. A successful
    /// response type is also a request which will be forwarded to the main service.
    pub fn expect<X1>(self, expect: X1) -> HttpService<T, S, B, X1, U>
    where
        X1: ServiceFactory<Request, Config = (), Response = Request>,
        X1::Error: Into<Response<BoxBody>>,
        X1::InitError: fmt::Debug,
    {
        HttpService {
            expect,
            cfg: self.cfg,
            srv: self.srv,
            upgrade: self.upgrade,
            on_connect_ext: self.on_connect_ext,
            _phantom: PhantomData,
        }
    }

    /// Sets service for custom `Connection: Upgrade` handling.
    ///
    /// If service is provided then normal requests handling get halted and this service get called
    /// with original request and framed object.
    pub fn upgrade<U1>(self, upgrade: Option<U1>) -> HttpService<T, S, B, X, U1>
    where
        U1: ServiceFactory<(Request, Framed<T, h1::Codec>), Config = (), Response = ()>,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
    {
        HttpService {
            upgrade,
            cfg: self.cfg,
            srv: self.srv,
            expect: self.expect,
            on_connect_ext: self.on_connect_ext,
            _phantom: PhantomData,
        }
    }

    /// Set connect callback with mutable access to request data container.
    pub(crate) fn on_connect_ext(mut self, f: Option<Rc<ConnectCallback<T>>>) -> Self {
        self.on_connect_ext = f;
        self
    }
}

impl<S, B, X, U> HttpService<TcpStream, S, B, X, U>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,

    B: MessageBody + 'static,

    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Response<BoxBody>>,
    X::InitError: fmt::Debug,

    U: ServiceFactory<(Request, Framed<TcpStream, h1::Codec>), Config = (), Response = ()>,
    U::Future: 'static,
    U::Error: fmt::Display + Into<Response<BoxBody>>,
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
        fn_service(|io: TcpStream| async {
            let peer_addr = io.peer_addr().ok();
            Ok((io, Protocol::Http1, peer_addr))
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

    impl<S, B, X, U> HttpService<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Response<BoxBody>>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, h1::Codec>),
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
                    let proto = if let Some(protos) = io.ssl().selected_alpn_protocol() {
                        if protos.windows(2).any(|window| window == b"h2") {
                            Protocol::Http2
                        } else {
                            Protocol::Http1
                        }
                    } else {
                        Protocol::Http1
                    };

                    let peer_addr = io.get_ref().peer_addr().ok();
                    (io, proto, peer_addr)
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls")]
mod rustls {
    use std::io;

    use actix_service::ServiceFactoryExt as _;
    use actix_tls::accept::{
        rustls::{reexports::ServerConfig, Acceptor, TlsStream},
        TlsError,
    };

    use super::*;

    impl<S, B, X, U> HttpService<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Response<BoxBody>>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, h1::Codec>),
            Config = (),
            Response = (),
        >,
        U::Future: 'static,
        U::Error: fmt::Display + Into<Response<BoxBody>>,
        U::InitError: fmt::Debug,
    {
        /// Create Rustls based service.
        pub fn rustls(
            self,
            mut config: ServerConfig,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = (),
        > {
            let mut protos = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            protos.extend_from_slice(&config.alpn_protocols);
            config.alpn_protocols = protos;

            Acceptor::new(config)
                .map_init_err(|_| {
                    unreachable!("TLS acceptor service factory does not error on init")
                })
                .map_err(TlsError::into_service_error)
                .and_then(|io: TlsStream<TcpStream>| async {
                    let proto = if let Some(protos) = io.get_ref().1.alpn_protocol() {
                        if protos.windows(2).any(|window| window == b"h2") {
                            Protocol::Http2
                        } else {
                            Protocol::Http1
                        }
                    } else {
                        Protocol::Http1
                    };
                    let peer_addr = io.get_ref().0.peer_addr().ok();
                    Ok((io, proto, peer_addr))
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

impl<T, S, B, X, U> ServiceFactory<(T, Protocol, Option<net::SocketAddr>)>
    for HttpService<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,

    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,

    B: MessageBody + 'static,

    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Response<BoxBody>>,
    X::InitError: fmt::Debug,

    U: ServiceFactory<(Request, Framed<T, h1::Codec>), Config = (), Response = ()>,
    U::Future: 'static,
    U::Error: fmt::Display + Into<Response<BoxBody>>,
    U::InitError: fmt::Debug,
{
    type Response = ();
    type Error = DispatchError;
    type Config = ();
    type Service = HttpServiceHandler<T, S::Service, B, X::Service, U::Service>;
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
                    let upgrade = upgrade
                        .await
                        .map_err(|e| log::error!("Init http upgrade service error: {:?}", e))?;
                    Some(upgrade)
                }
                None => None,
            };

            let service = service
                .await
                .map_err(|e| log::error!("Init http service error: {:?}", e))?;

            Ok(HttpServiceHandler::new(
                cfg,
                service,
                expect,
                upgrade,
                on_connect_ext,
            ))
        })
    }
}

/// `Service` implementation for HTTP/1 and HTTP/2 transport
pub struct HttpServiceHandler<T, S, B, X, U>
where
    S: Service<Request>,
    X: Service<Request>,
    U: Service<(Request, Framed<T, h1::Codec>)>,
{
    pub(super) flow: Rc<HttpFlow<S, X, U>>,
    pub(super) cfg: ServiceConfig,
    pub(super) on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _phantom: PhantomData<B>,
}

impl<T, S, B, X, U> HttpServiceHandler<T, S, B, X, U>
where
    S: Service<Request>,
    S::Error: Into<Response<BoxBody>>,
    X: Service<Request>,
    X::Error: Into<Response<BoxBody>>,
    U: Service<(Request, Framed<T, h1::Codec>)>,
    U::Error: Into<Response<BoxBody>>,
{
    pub(super) fn new(
        cfg: ServiceConfig,
        service: S,
        expect: X,
        upgrade: Option<U>,
        on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    ) -> HttpServiceHandler<T, S, B, X, U> {
        HttpServiceHandler {
            cfg,
            on_connect_ext,
            flow: HttpFlow::new(service, expect, upgrade),
            _phantom: PhantomData,
        }
    }

    pub(super) fn _poll_ready(
        &self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Response<BoxBody>>> {
        ready!(self.flow.expect.poll_ready(cx).map_err(Into::into))?;

        ready!(self.flow.service.poll_ready(cx).map_err(Into::into))?;

        if let Some(ref upg) = self.flow.upgrade {
            ready!(upg.poll_ready(cx).map_err(Into::into))?;
        };

        Poll::Ready(Ok(()))
    }
}

/// A collection of services that describe an HTTP request flow.
pub(super) struct HttpFlow<S, X, U> {
    pub(super) service: S,
    pub(super) expect: X,
    pub(super) upgrade: Option<U>,
}

impl<S, X, U> HttpFlow<S, X, U> {
    pub(super) fn new(service: S, expect: X, upgrade: Option<U>) -> Rc<Self> {
        Rc::new(Self {
            service,
            expect,
            upgrade,
        })
    }
}

impl<T, S, B, X, U> Service<(T, Protocol, Option<net::SocketAddr>)>
    for HttpServiceHandler<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,

    S: Service<Request>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,

    B: MessageBody + 'static,

    X: Service<Request, Response = Request>,
    X::Error: Into<Response<BoxBody>>,

    U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display + Into<Response<BoxBody>>,
{
    type Response = ();
    type Error = DispatchError;
    type Future = HttpServiceHandlerResponse<T, S, B, X, U>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self._poll_ready(cx).map_err(|err| {
            log::error!("HTTP service readiness error: {:?}", err);
            DispatchError::Service(err)
        })
    }

    fn call(
        &self,
        (io, proto, peer_addr): (T, Protocol, Option<net::SocketAddr>),
    ) -> Self::Future {
        let conn_data = OnConnectData::from_io(&io, self.on_connect_ext.as_deref());

        match proto {
            #[cfg(feature = "http2")]
            Protocol::Http2 => HttpServiceHandlerResponse {
                state: State::H2Handshake {
                    handshake: Some((
                        crate::h2::handshake_with_timeout(io, &self.cfg),
                        self.cfg.clone(),
                        self.flow.clone(),
                        conn_data,
                        peer_addr,
                    )),
                },
            },

            #[cfg(not(feature = "http2"))]
            Protocol::Http2 => {
                panic!("HTTP/2 support is disabled (enable with the `http2` feature flag)")
            }

            Protocol::Http1 => HttpServiceHandlerResponse {
                state: State::H1 {
                    dispatcher: h1::Dispatcher::new(
                        io,
                        self.flow.clone(),
                        self.cfg.clone(),
                        peer_addr,
                        conn_data,
                    ),
                },
            },

            proto => unimplemented!("Unsupported HTTP version: {:?}.", proto),
        }
    }
}

#[cfg(not(feature = "http2"))]
pin_project! {
    #[project = StateProj]
    enum State<T, S, B, X, U>
    where
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,

        S: Service<Request>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>>,

        B: MessageBody,

        X: Service<Request, Response = Request>,
        X::Error: Into<Response<BoxBody>>,

        U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
        U::Error: fmt::Display,
    {
        H1 { #[pin] dispatcher: h1::Dispatcher<T, S, B, X, U> },
    }
}

#[cfg(feature = "http2")]
pin_project! {
    #[project = StateProj]
    enum State<T, S, B, X, U>
    where
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,

        S: Service<Request>,
        S::Future: 'static,
        S::Error: Into<Response<BoxBody>>,

        B: MessageBody,

        X: Service<Request, Response = Request>,
        X::Error: Into<Response<BoxBody>>,

        U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
        U::Error: fmt::Display,
    {
        H1 { #[pin] dispatcher: h1::Dispatcher<T, S, B, X, U> },

        H2 { #[pin] dispatcher: crate::h2::Dispatcher<T, S, B, X, U> },

        H2Handshake {
            handshake: Option<(
                crate::h2::HandshakeWithTimeout<T>,
                ServiceConfig,
                Rc<HttpFlow<S, X, U>>,
                OnConnectData,
                Option<net::SocketAddr>,
            )>,
        },
    }
}

pin_project! {
    pub struct HttpServiceHandlerResponse<T, S, B, X, U>
    where
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,

        S: Service<Request>,
        S::Error: Into<Response<BoxBody>>,
        S::Error: 'static,
        S::Future: 'static,
        S::Response: Into<Response<B>>,
        S::Response: 'static,

        B: MessageBody,

        X: Service<Request, Response = Request>,
        X::Error: Into<Response<BoxBody>>,

        U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
        U::Error: fmt::Display,
    {
        #[pin]
        state: State<T, S, B, X, U>,
    }
}

impl<T, S, B, X, U> Future for HttpServiceHandlerResponse<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,

    S: Service<Request>,
    S::Error: Into<Response<BoxBody>> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,

    B: MessageBody + 'static,

    X: Service<Request, Response = Request>,
    X::Error: Into<Response<BoxBody>>,

    U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Output = Result<(), DispatchError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project().state.project() {
            StateProj::H1 { dispatcher } => dispatcher.poll(cx),

            #[cfg(feature = "http2")]
            StateProj::H2 { dispatcher } => dispatcher.poll(cx),

            #[cfg(feature = "http2")]
            StateProj::H2Handshake { handshake: data } => {
                match ready!(Pin::new(&mut data.as_mut().unwrap().0).poll(cx)) {
                    Ok((conn, timer)) => {
                        let (_, config, flow, conn_data, peer_addr) = data.take().unwrap();

                        self.as_mut().project().state.set(State::H2 {
                            dispatcher: crate::h2::Dispatcher::new(
                                conn, flow, config, peer_addr, conn_data, timer,
                            ),
                        });
                        self.poll(cx)
                    }
                    Err(err) => {
                        log::trace!("H2 handshake error: {}", err);
                        Poll::Ready(Err(err))
                    }
                }
            }
        }
    }
}

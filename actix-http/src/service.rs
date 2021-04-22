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
use bytes::Bytes;
use futures_core::{future::LocalBoxFuture, ready};
use h2::server::{handshake, Handshake};
use pin_project::pin_project;

use crate::body::MessageBody;
use crate::builder::HttpServiceBuilder;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, Error};
use crate::request::Request;
use crate::response::Response;
use crate::{h1, h2::Dispatcher, ConnectCallback, OnConnectData, Protocol};

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
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create builder for `HttpService` instance.
    pub fn build() -> HttpServiceBuilder<T, S> {
        HttpServiceBuilder::new()
    }
}

impl<T, S, B> HttpService<T, S, B>
where
    S: ServiceFactory<Request, Config = ()>,
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoServiceFactory<S, Request>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0, false, None);

        HttpService {
            cfg,
            srv: service.into_factory(),
            expect: h1::ExpectHandler,
            upgrade: None,
            on_connect_ext: None,
            _phantom: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
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
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    B: MessageBody,
{
    /// Provide service for `EXPECT: 100-Continue` support.
    ///
    /// Service get called with request that contains `EXPECT` header.
    /// Service must return request in case of success, in that case
    /// request will be forwarded to main service.
    pub fn expect<X1>(self, expect: X1) -> HttpService<T, S, B, X1, U>
    where
        X1: ServiceFactory<Request, Config = (), Response = Request>,
        X1::Error: Into<Error>,
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

    /// Provide service for custom `Connection: UPGRADE` support.
    ///
    /// If service is provided then normal requests handling get halted
    /// and this service get called with original request and framed object.
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
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,

    B: MessageBody + 'static,

    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,

    U: ServiceFactory<
        (Request, Framed<TcpStream, h1::Codec>),
        Config = (),
        Response = (),
    >,
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
        fn_service(|io: TcpStream| async {
            let peer_addr = io.peer_addr().ok();
            Ok((io, Protocol::Http1, peer_addr))
        })
        .and_then(self)
    }
}

#[cfg(feature = "openssl")]
mod openssl {
    use actix_service::ServiceFactoryExt;
    use actix_tls::accept::openssl::{Acceptor, SslAcceptor, SslError, TlsStream};
    use actix_tls::accept::TlsError;

    use super::*;

    impl<S, B, X, U> HttpService<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Error> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Error>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, h1::Codec>),
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
                .and_then(|io: TlsStream<TcpStream>| async {
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
                    Ok((io, proto, peer_addr))
                })
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls")]
mod rustls {
    use std::io;

    use actix_tls::accept::rustls::{Acceptor, ServerConfig, Session, TlsStream};
    use actix_tls::accept::TlsError;

    use super::*;
    use actix_service::ServiceFactoryExt;

    impl<S, B, X, U> HttpService<TlsStream<TcpStream>, S, B, X, U>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Error> + 'static,
        S::InitError: fmt::Debug,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,

        B: MessageBody + 'static,

        X: ServiceFactory<Request, Config = (), Response = Request>,
        X::Future: 'static,
        X::Error: Into<Error>,
        X::InitError: fmt::Debug,

        U: ServiceFactory<
            (Request, Framed<TlsStream<TcpStream>, h1::Codec>),
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
            mut config: ServerConfig,
        ) -> impl ServiceFactory<
            TcpStream,
            Config = (),
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = (),
        > {
            let protos = vec!["h2".to_string().into(), "http/1.1".to_string().into()];
            config.set_protocols(&protos);

            Acceptor::new(config)
                .map_err(TlsError::Tls)
                .map_init_err(|_| panic!())
                .and_then(|io: TlsStream<TcpStream>| async {
                    let proto = if let Some(protos) = io.get_ref().1.get_alpn_protocol()
                    {
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
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,

    B: MessageBody + 'static,

    X: ServiceFactory<Request, Config = (), Response = Request>,
    X::Future: 'static,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,

    U: ServiceFactory<(Request, Framed<T, h1::Codec>), Config = (), Response = ()>,
    U::Future: 'static,
    U::Error: fmt::Display + Into<Error>,
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
    S::Error: Into<Error>,
    X: Service<Request>,
    X::Error: Into<Error>,
    U: Service<(Request, Framed<T, h1::Codec>)>,
    U::Error: Into<Error>,
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

    pub(super) fn _poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
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
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
    X: Service<Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display + Into<Error>,
{
    type Response = ();
    type Error = DispatchError;
    type Future = HttpServiceHandlerResponse<T, S, B, X, U>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self._poll_ready(cx).map_err(|e| {
            log::error!("HTTP service readiness error: {:?}", e);
            DispatchError::Service(e)
        })
    }

    fn call(
        &self,
        (io, proto, peer_addr): (T, Protocol, Option<net::SocketAddr>),
    ) -> Self::Future {
        let on_connect_data =
            OnConnectData::from_io(&io, self.on_connect_ext.as_deref());

        match proto {
            Protocol::Http2 => HttpServiceHandlerResponse {
                state: State::H2Handshake(Some((
                    handshake(io),
                    self.cfg.clone(),
                    self.flow.clone(),
                    on_connect_data,
                    peer_addr,
                ))),
            },

            Protocol::Http1 => HttpServiceHandlerResponse {
                state: State::H1(h1::Dispatcher::new(
                    io,
                    self.cfg.clone(),
                    self.flow.clone(),
                    on_connect_data,
                    peer_addr,
                )),
            },

            proto => unimplemented!("Unsupported HTTP version: {:?}.", proto),
        }
    }
}

#[pin_project(project = StateProj)]
enum State<T, S, B, X, U>
where
    S: Service<Request>,
    S::Future: 'static,
    S::Error: Into<Error>,
    T: AsyncRead + AsyncWrite + Unpin,
    B: MessageBody,
    X: Service<Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    H1(#[pin] h1::Dispatcher<T, S, B, X, U>),
    H2(#[pin] Dispatcher<T, S, B, X, U>),
    H2Handshake(
        Option<(
            Handshake<T, Bytes>,
            ServiceConfig,
            Rc<HttpFlow<S, X, U>>,
            OnConnectData,
            Option<net::SocketAddr>,
        )>,
    ),
}

#[pin_project]
pub struct HttpServiceHandlerResponse<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
    X: Service<Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    #[pin]
    state: State<T, S, B, X, U>,
}

impl<T, S, B, X, U> Future for HttpServiceHandlerResponse<T, S, B, X, U>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody,
    X: Service<Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<(Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Output = Result<(), DispatchError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project().state.project() {
            StateProj::H1(disp) => disp.poll(cx),
            StateProj::H2(disp) => disp.poll(cx),
            StateProj::H2Handshake(data) => {
                match ready!(Pin::new(&mut data.as_mut().unwrap().0).poll(cx)) {
                    Ok(conn) => {
                        let (_, cfg, srv, on_connect_data, peer_addr) =
                            data.take().unwrap();
                        self.as_mut().project().state.set(State::H2(Dispatcher::new(
                            srv,
                            conn,
                            on_connect_data,
                            cfg,
                            peer_addr,
                        )));
                        self.poll(cx)
                    }
                    Err(err) => {
                        trace!("H2 handshake error: {}", err);
                        Poll::Ready(Err(err.into()))
                    }
                }
            }
        }
    }
}

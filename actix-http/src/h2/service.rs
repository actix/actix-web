use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{net, rc::Rc};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::net::TcpStream;
use actix_service::{
    fn_factory, fn_service, IntoServiceFactory, Service, ServiceFactory,
    ServiceFactoryExt as _,
};
use actix_utils::future::ready;
use bytes::Bytes;
use futures_core::{future::LocalBoxFuture, ready};
use h2::server::{handshake, Handshake};
use log::error;

use crate::body::MessageBody;
use crate::config::ServiceConfig;
use crate::error::{DispatchError, Error};
use crate::request::Request;
use crate::response::Response;
use crate::service::HttpFlow;
use crate::{ConnectCallback, OnConnectData};

use super::dispatcher::Dispatcher;

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
    S::Error: Into<Error> + 'static,
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
    S::Error: Into<Error> + 'static,
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
    use actix_service::{fn_factory, fn_service, ServiceFactoryExt};
    use actix_tls::accept::openssl::{Acceptor, SslAcceptor, SslError, TlsStream};
    use actix_tls::accept::TlsError;

    use super::*;

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Error> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,
        B: MessageBody + 'static,
    {
        /// Create OpenSSL based service
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
                .map_err(TlsError::Tls)
                .map_init_err(|_| panic!())
                .and_then(fn_factory(|| {
                    ready(Ok::<_, S::InitError>(fn_service(
                        |io: TlsStream<TcpStream>| {
                            let peer_addr = io.get_ref().peer_addr().ok();
                            ready(Ok((io, peer_addr)))
                        },
                    )))
                }))
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls")]
mod rustls {
    use super::*;
    use actix_service::ServiceFactoryExt;
    use actix_tls::accept::rustls::{Acceptor, ServerConfig, TlsStream};
    use actix_tls::accept::TlsError;
    use std::io;

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Request, Config = ()>,
        S::Future: 'static,
        S::Error: Into<Error> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service<Request>>::Future: 'static,
        B: MessageBody + 'static,
    {
        /// Create Rustls based service
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
            let protos = vec!["h2".to_string().into()];
            config.set_protocols(&protos);

            Acceptor::new(config)
                .map_err(TlsError::Tls)
                .map_init_err(|_| panic!())
                .and_then(fn_factory(|| {
                    ready(Ok::<_, S::InitError>(fn_service(
                        |io: TlsStream<TcpStream>| {
                            let peer_addr = io.get_ref().0.peer_addr().ok();
                            ready(Ok((io, peer_addr)))
                        },
                    )))
                }))
                .and_then(self.map_err(TlsError::Service))
        }
    }
}

impl<T, S, B> ServiceFactory<(T, Option<net::SocketAddr>)> for H2Service<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
    S: ServiceFactory<Request, Config = ()>,
    S::Future: 'static,
    S::Error: Into<Error> + 'static,
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
    S::Error: Into<Error> + 'static,
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
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    type Response = ();
    type Error = DispatchError;
    type Future = H2ServiceHandlerResponse<T, S, B>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.flow.service.poll_ready(cx).map_err(|e| {
            let e = e.into();
            error!("Service readiness error: {:?}", e);
            DispatchError::Service(e)
        })
    }

    fn call(&self, (io, addr): (T, Option<net::SocketAddr>)) -> Self::Future {
        let on_connect_data =
            OnConnectData::from_io(&io, self.on_connect_ext.as_deref());

        H2ServiceHandlerResponse {
            state: State::Handshake(
                Some(self.flow.clone()),
                Some(self.cfg.clone()),
                addr,
                on_connect_data,
                handshake(io),
            ),
        }
    }
}

enum State<T, S: Service<Request>, B: MessageBody>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S::Future: 'static,
{
    Incoming(Dispatcher<T, S, B, (), ()>),
    Handshake(
        Option<Rc<HttpFlow<S, (), ()>>>,
        Option<ServiceConfig>,
        Option<net::SocketAddr>,
        OnConnectData,
        Handshake<T, Bytes>,
    ),
}

pub struct H2ServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request>,
    S::Error: Into<Error> + 'static,
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
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody,
{
    type Output = Result<(), DispatchError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.state {
            State::Incoming(ref mut disp) => Pin::new(disp).poll(cx),
            State::Handshake(
                ref mut srv,
                ref mut config,
                ref peer_addr,
                ref mut on_connect_data,
                ref mut handshake,
            ) => match ready!(Pin::new(handshake).poll(cx)) {
                Ok(conn) => {
                    let on_connect_data = std::mem::take(on_connect_data);
                    self.state = State::Incoming(Dispatcher::new(
                        srv.take().unwrap(),
                        conn,
                        on_connect_data,
                        config.take().unwrap(),
                        *peer_addr,
                    ));
                    self.poll(cx)
                }
                Err(err) => {
                    trace!("H2 handshake error: {}", err);
                    Poll::Ready(Err(err.into()))
                }
            },
        }
    }
}

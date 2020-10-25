use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{net, rc::Rc};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::net::TcpStream;
use actix_service::{
    fn_factory, fn_service, pipeline_factory, IntoServiceFactory, Service,
    ServiceFactory,
};
use bytes::Bytes;
use futures_core::ready;
use futures_util::future::ok;
use h2::server::{self, Handshake};
use log::error;

use crate::body::MessageBody;
use crate::cloneable::CloneableService;
use crate::config::ServiceConfig;
use crate::error::{DispatchError, Error};
use crate::helpers::DataFactory;
use crate::request::Request;
use crate::response::Response;
use crate::{ConnectCallback, Extensions};

use super::dispatcher::Dispatcher;

/// `ServiceFactory` implementation for HTTP2 transport
pub struct H2Service<T, S, B> {
    srv: S,
    cfg: ServiceConfig,
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> H2Service<T, S, B>
where
    S: ServiceFactory<Config = (), Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance with config.
    pub(crate) fn with_config<F: IntoServiceFactory<S>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        H2Service {
            cfg,
            on_connect: None,
            on_connect_ext: None,
            srv: service.into_factory(),
            _t: PhantomData,
        }
    }

    /// Set on connect callback.

    pub(crate) fn on_connect(
        mut self,
        f: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    ) -> Self {
        self.on_connect = f;
        self
    }

    /// Set on connect callback.
    pub(crate) fn on_connect_ext(mut self, f: Option<Rc<ConnectCallback<T>>>) -> Self {
        self.on_connect_ext = f;
        self
    }
}

impl<S, B> H2Service<TcpStream, S, B>
where
    S: ServiceFactory<Config = (), Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create simple tcp based service
    pub fn tcp(
        self,
    ) -> impl ServiceFactory<
        Config = (),
        Request = TcpStream,
        Response = (),
        Error = DispatchError,
        InitError = S::InitError,
    > {
        pipeline_factory(fn_factory(|| async {
            Ok::<_, S::InitError>(fn_service(|io: TcpStream| {
                let peer_addr = io.peer_addr().ok();
                ok::<_, DispatchError>((io, peer_addr))
            }))
        }))
        .and_then(self)
    }
}

#[cfg(feature = "openssl")]
mod openssl {
    use actix_service::{fn_factory, fn_service};
    use actix_tls::openssl::{Acceptor, SslAcceptor, SslStream};
    use actix_tls::{openssl::HandshakeError, TlsError};

    use super::*;

    impl<S, B> H2Service<SslStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Config = (), Request = Request>,
        S::Error: Into<Error> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service>::Future: 'static,
        B: MessageBody + 'static,
    {
        /// Create ssl based service
        pub fn openssl(
            self,
            acceptor: SslAcceptor,
        ) -> impl ServiceFactory<
            Config = (),
            Request = TcpStream,
            Response = (),
            Error = TlsError<HandshakeError<TcpStream>, DispatchError>,
            InitError = S::InitError,
        > {
            pipeline_factory(
                Acceptor::new(acceptor)
                    .map_err(TlsError::Tls)
                    .map_init_err(|_| panic!()),
            )
            .and_then(fn_factory(|| {
                ok::<_, S::InitError>(fn_service(|io: SslStream<TcpStream>| {
                    let peer_addr = io.get_ref().peer_addr().ok();
                    ok((io, peer_addr))
                }))
            }))
            .and_then(self.map_err(TlsError::Service))
        }
    }
}

#[cfg(feature = "rustls")]
mod rustls {
    use super::*;
    use actix_tls::rustls::{Acceptor, ServerConfig, TlsStream};
    use actix_tls::TlsError;
    use std::io;

    impl<S, B> H2Service<TlsStream<TcpStream>, S, B>
    where
        S: ServiceFactory<Config = (), Request = Request>,
        S::Error: Into<Error> + 'static,
        S::Response: Into<Response<B>> + 'static,
        <S::Service as Service>::Future: 'static,
        B: MessageBody + 'static,
    {
        /// Create openssl based service
        pub fn rustls(
            self,
            mut config: ServerConfig,
        ) -> impl ServiceFactory<
            Config = (),
            Request = TcpStream,
            Response = (),
            Error = TlsError<io::Error, DispatchError>,
            InitError = S::InitError,
        > {
            let protos = vec!["h2".to_string().into()];
            config.set_protocols(&protos);

            pipeline_factory(
                Acceptor::new(config)
                    .map_err(TlsError::Tls)
                    .map_init_err(|_| panic!()),
            )
            .and_then(fn_factory(|| {
                ok::<_, S::InitError>(fn_service(|io: TlsStream<TcpStream>| {
                    let peer_addr = io.get_ref().0.peer_addr().ok();
                    ok((io, peer_addr))
                }))
            }))
            .and_then(self.map_err(TlsError::Service))
        }
    }
}

impl<T, S, B> ServiceFactory for H2Service<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: ServiceFactory<Config = (), Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    type Config = ();
    type Request = (T, Option<net::SocketAddr>);
    type Response = ();
    type Error = DispatchError;
    type InitError = S::InitError;
    type Service = H2ServiceHandler<T, S::Service, B>;
    type Future = H2ServiceResponse<T, S, B>;

    fn new_service(&self, _: ()) -> Self::Future {
        H2ServiceResponse {
            fut: self.srv.new_service(()),
            cfg: Some(self.cfg.clone()),
            on_connect: self.on_connect.clone(),
            on_connect_ext: self.on_connect_ext.clone(),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
#[pin_project::pin_project]
pub struct H2ServiceResponse<T, S: ServiceFactory, B> {
    #[pin]
    fut: S::Future,
    cfg: Option<ServiceConfig>,
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> Future for H2ServiceResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: ServiceFactory<Config = (), Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    type Output = Result<H2ServiceHandler<T, S::Service, B>, S::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        Poll::Ready(ready!(this.fut.poll(cx)).map(|service| {
            let this = self.as_mut().project();
            H2ServiceHandler::new(
                this.cfg.take().unwrap(),
                this.on_connect.clone(),
                this.on_connect_ext.clone(),
                service,
            )
        }))
    }
}

/// `Service` implementation for http/2 transport
pub struct H2ServiceHandler<T, S: Service, B> {
    srv: CloneableService<S>,
    cfg: ServiceConfig,
    on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    on_connect_ext: Option<Rc<ConnectCallback<T>>>,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> H2ServiceHandler<T, S, B>
where
    S: Service<Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    fn new(
        cfg: ServiceConfig,
        on_connect: Option<Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
        on_connect_ext: Option<Rc<ConnectCallback<T>>>,
        srv: S,
    ) -> H2ServiceHandler<T, S, B> {
        H2ServiceHandler {
            cfg,
            on_connect,
            on_connect_ext,
            srv: CloneableService::new(srv),
            _t: PhantomData,
        }
    }
}

impl<T, S, B> Service for H2ServiceHandler<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    type Request = (T, Option<net::SocketAddr>);
    type Response = ();
    type Error = DispatchError;
    type Future = H2ServiceHandlerResponse<T, S, B>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.srv.poll_ready(cx).map_err(|e| {
            let e = e.into();
            error!("Service readiness error: {:?}", e);
            DispatchError::Service(e)
        })
    }

    fn call(&mut self, (io, addr): Self::Request) -> Self::Future {
        let deprecated_on_connect = self.on_connect.as_ref().map(|handler| handler(&io));

        let mut connect_extensions = Extensions::new();
        if let Some(ref handler) = self.on_connect_ext {
            // run on_connect_ext callback, populating connect extensions
            handler(&io, &mut connect_extensions);
        }

        H2ServiceHandlerResponse {
            state: State::Handshake(
                Some(self.srv.clone()),
                Some(self.cfg.clone()),
                addr,
                deprecated_on_connect,
                Some(connect_extensions),
                server::handshake(io),
            ),
        }
    }
}

enum State<T, S: Service<Request = Request>, B: MessageBody>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S::Future: 'static,
{
    Incoming(Dispatcher<T, S, B>),
    Handshake(
        Option<CloneableService<S>>,
        Option<ServiceConfig>,
        Option<net::SocketAddr>,
        Option<Box<dyn DataFactory>>,
        Option<Extensions>,
        Handshake<T, Bytes>,
    ),
}

pub struct H2ServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: Service<Request = Request>,
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
    S: Service<Request = Request>,
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
                ref mut on_connect,
                ref mut on_connect_data,
                ref mut handshake,
            ) => match Pin::new(handshake).poll(cx) {
                Poll::Ready(Ok(conn)) => {
                    self.state = State::Incoming(Dispatcher::new(
                        srv.take().unwrap(),
                        conn,
                        on_connect.take(),
                        on_connect_data.take().unwrap(),
                        config.take().unwrap(),
                        None,
                        *peer_addr,
                    ));
                    self.poll(cx)
                }
                Poll::Ready(Err(err)) => {
                    trace!("H2 handshake error: {}", err);
                    Poll::Ready(Err(err.into()))
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

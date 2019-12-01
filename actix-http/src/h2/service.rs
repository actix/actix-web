use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{io, net, rc};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_server_config::{Io, IoStream, ServerConfig as SrvConfig};
use actix_service::{IntoServiceFactory, Service, ServiceFactory};
use bytes::Bytes;
use futures::future::{ok, Ready};
use futures::{ready, Stream};
use h2::server::{self, Connection, Handshake};
use h2::RecvStream;
use log::error;

use crate::body::MessageBody;
use crate::cloneable::CloneableService;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, Error, ParseError, ResponseError};
use crate::helpers::DataFactory;
use crate::payload::Payload;
use crate::request::Request;
use crate::response::Response;

use super::dispatcher::Dispatcher;

/// `ServiceFactory` implementation for HTTP2 transport
pub struct H2Service<T, P, S, B> {
    srv: S,
    cfg: ServiceConfig,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> H2Service<T, P, S, B>
where
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoServiceFactory<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H2Service {
            cfg,
            on_connect: None,
            srv: service.into_factory(),
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub fn with_config<F: IntoServiceFactory<S>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        H2Service {
            cfg,
            on_connect: None,
            srv: service.into_factory(),
            _t: PhantomData,
        }
    }

    /// Set on connect callback.
    pub(crate) fn on_connect(
        mut self,
        f: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    ) -> Self {
        self.on_connect = f;
        self
    }
}

impl<T, P, S, B> ServiceFactory for H2Service<T, P, S, B>
where
    T: IoStream,
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    type Config = SrvConfig;
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type InitError = S::InitError;
    type Service = H2ServiceHandler<T, P, S::Service, B>;
    type Future = H2ServiceResponse<T, P, S, B>;

    fn new_service(&self, cfg: &SrvConfig) -> Self::Future {
        H2ServiceResponse {
            fut: self.srv.new_service(cfg),
            cfg: Some(self.cfg.clone()),
            on_connect: self.on_connect.clone(),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
#[pin_project::pin_project]
pub struct H2ServiceResponse<T, P, S: ServiceFactory, B> {
    #[pin]
    fut: S::Future,
    cfg: Option<ServiceConfig>,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> Future for H2ServiceResponse<T, P, S, B>
where
    T: IoStream,
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    type Output = Result<H2ServiceHandler<T, P, S::Service, B>, S::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        Poll::Ready(ready!(this.fut.poll(cx)).map(|service| {
            let this = self.as_mut().project();
            H2ServiceHandler::new(
                this.cfg.take().unwrap(),
                this.on_connect.clone(),
                service,
            )
        }))
    }
}

/// `Service` implementation for http/2 transport
pub struct H2ServiceHandler<T, P, S, B> {
    srv: CloneableService<S>,
    cfg: ServiceConfig,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> H2ServiceHandler<T, P, S, B>
where
    S: Service<Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    fn new(
        cfg: ServiceConfig,
        on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
        srv: S,
    ) -> H2ServiceHandler<T, P, S, B> {
        H2ServiceHandler {
            cfg,
            on_connect,
            srv: CloneableService::new(srv),
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B> Service for H2ServiceHandler<T, P, S, B>
where
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody + 'static,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = H2ServiceHandlerResponse<T, S, B>;

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.srv.poll_ready(cx).map_err(|e| {
            let e = e.into();
            error!("Service readiness error: {:?}", e);
            DispatchError::Service(e)
        })
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let io = req.into_parts().0;
        let peer_addr = io.peer_addr();
        let on_connect = if let Some(ref on_connect) = self.on_connect {
            Some(on_connect(&io))
        } else {
            None
        };

        H2ServiceHandlerResponse {
            state: State::Handshake(
                Some(self.srv.clone()),
                Some(self.cfg.clone()),
                peer_addr,
                on_connect,
                server::handshake(io),
            ),
        }
    }
}

enum State<T: IoStream, S: Service<Request = Request>, B: MessageBody>
where
    S::Future: 'static,
{
    Incoming(Dispatcher<T, S, B>),
    Handshake(
        Option<CloneableService<S>>,
        Option<ServiceConfig>,
        Option<net::SocketAddr>,
        Option<Box<dyn DataFactory>>,
        Handshake<T, Bytes>,
    ),
}

pub struct H2ServiceHandlerResponse<T, S, B>
where
    T: IoStream,
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
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error> + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + 'static,
    B: MessageBody,
{
    type Output = Result<(), DispatchError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match self.state {
            State::Incoming(ref mut disp) => Pin::new(disp).poll(cx),
            State::Handshake(
                ref mut srv,
                ref mut config,
                ref peer_addr,
                ref mut on_connect,
                ref mut handshake,
            ) => match Pin::new(handshake).poll(cx) {
                Poll::Ready(Ok(conn)) => {
                    self.state = State::Incoming(Dispatcher::new(
                        srv.take().unwrap(),
                        conn,
                        on_connect.take(),
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

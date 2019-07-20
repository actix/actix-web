use std::fmt::Debug;
use std::marker::PhantomData;
use std::{io, net, rc};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_server_config::{Io, IoStream, ServerConfig as SrvConfig};
use actix_service::{IntoNewService, NewService, Service};
use bytes::Bytes;
use futures::future::{ok, FutureResult};
use futures::{try_ready, Async, Future, IntoFuture, Poll, Stream};
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

/// `NewService` implementation for HTTP2 transport
pub struct H2Service<T, P, S, B> {
    srv: S,
    cfg: ServiceConfig,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> H2Service<T, P, S, B>
where
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H2Service {
            cfg,
            on_connect: None,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub fn with_config<F: IntoNewService<S>>(cfg: ServiceConfig, service: F) -> Self {
        H2Service {
            cfg,
            on_connect: None,
            srv: service.into_new_service(),
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

impl<T, P, S, B> NewService for H2Service<T, P, S, B>
where
    T: IoStream,
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
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
            fut: self.srv.new_service(cfg).into_future(),
            cfg: Some(self.cfg.clone()),
            on_connect: self.on_connect.clone(),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct H2ServiceResponse<T, P, S: NewService, B> {
    fut: <S::Future as IntoFuture>::Future,
    cfg: Option<ServiceConfig>,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> Future for H2ServiceResponse<T, P, S, B>
where
    T: IoStream,
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    type Item = H2ServiceHandler<T, P, S::Service, B>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let service = try_ready!(self.fut.poll());
        Ok(Async::Ready(H2ServiceHandler::new(
            self.cfg.take().unwrap(),
            self.on_connect.clone(),
            service,
        )))
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
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
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
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = H2ServiceHandlerResponse<T, S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.srv.poll_ready().map_err(|e| {
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
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    state: State<T, S, B>,
}

impl<T, S, B> Future for H2ServiceHandlerResponse<T, S, B>
where
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    type Item = ();
    type Error = DispatchError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.state {
            State::Incoming(ref mut disp) => disp.poll(),
            State::Handshake(
                ref mut srv,
                ref mut config,
                ref peer_addr,
                ref mut on_connect,
                ref mut handshake,
            ) => match handshake.poll() {
                Ok(Async::Ready(conn)) => {
                    self.state = State::Incoming(Dispatcher::new(
                        srv.take().unwrap(),
                        conn,
                        on_connect.take(),
                        config.take().unwrap(),
                        None,
                        *peer_addr,
                    ));
                    self.poll()
                }
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(err) => {
                    trace!("H2 handshake error: {}", err);
                    Err(err.into())
                }
            },
        }
    }
}

use std::fmt::Debug;
use std::marker::PhantomData;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_server_config::{Io, ServerConfig as SrvConfig};
use actix_service::{IntoNewService, NewService, Service};
use actix_utils::cloneable::CloneableService;
use futures::future::{ok, FutureResult};
use futures::{try_ready, Async, Future, IntoFuture, Poll, Stream};

use crate::body::MessageBody;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, ParseError};
use crate::request::Request;
use crate::response::Response;

use super::codec::Codec;
use super::dispatcher::Dispatcher;
use super::Message;

/// `NewService` implementation for HTTP1 transport
pub struct H1Service<T, P, S, B> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> H1Service<T, P, S, B>
where
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    S::Service: 'static,
    B: MessageBody,
{
    /// Create new `HttpService` instance with default config.
    pub fn new<F: IntoNewService<S, SrvConfig>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H1Service {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub fn with_config<F: IntoNewService<S, SrvConfig>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        H1Service {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B> NewService<SrvConfig> for H1Service<T, P, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    S::Service: 'static,
    B: MessageBody,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type InitError = S::InitError;
    type Service = H1ServiceHandler<T, P, S::Service, B>;
    type Future = H1ServiceResponse<T, P, S, B>;

    fn new_service(&self, cfg: &SrvConfig) -> Self::Future {
        H1ServiceResponse {
            fut: self.srv.new_service(cfg).into_future(),
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct H1ServiceResponse<T, P, S: NewService<SrvConfig, Request = Request>, B> {
    fut: <S::Future as IntoFuture>::Future,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> Future for H1ServiceResponse<T, P, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<SrvConfig, Request = Request>,
    S::Service: 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    type Item = H1ServiceHandler<T, P, S::Service, B>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let service = try_ready!(self.fut.poll());
        Ok(Async::Ready(H1ServiceHandler::new(
            self.cfg.take().unwrap(),
            service,
        )))
    }
}

/// `Service` implementation for HTTP1 transport
pub struct H1ServiceHandler<T, P, S: 'static, B> {
    srv: CloneableService<S>,
    cfg: ServiceConfig,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> H1ServiceHandler<T, P, S, B>
where
    S: Service<Request = Request>,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    fn new(cfg: ServiceConfig, srv: S) -> H1ServiceHandler<T, P, S, B> {
        H1ServiceHandler {
            srv: CloneableService::new(srv),
            cfg,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B> Service for H1ServiceHandler<T, P, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request>,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = Dispatcher<T, S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.srv.poll_ready().map_err(|e| {
            log::error!("Http service readiness error: {:?}", e);
            DispatchError::Service
        })
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        Dispatcher::new(req.into_parts().0, self.cfg.clone(), self.srv.clone())
    }
}

/// `NewService` implementation for `OneRequestService` service
#[derive(Default)]
pub struct OneRequest<T, P> {
    config: ServiceConfig,
    _t: PhantomData<(T, P)>,
}

impl<T, P> OneRequest<T, P>
where
    T: AsyncRead + AsyncWrite,
{
    /// Create new `H1SimpleService` instance.
    pub fn new() -> Self {
        OneRequest {
            config: ServiceConfig::default(),
            _t: PhantomData,
        }
    }
}

impl<T, P> NewService<SrvConfig> for OneRequest<T, P>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = Io<T, P>;
    type Response = (Request, Framed<T, Codec>);
    type Error = ParseError;
    type InitError = ();
    type Service = OneRequestService<T, P>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &SrvConfig) -> Self::Future {
        ok(OneRequestService {
            config: self.config.clone(),
            _t: PhantomData,
        })
    }
}

/// `Service` implementation for HTTP1 transport. Reads one request and returns
/// request and framed object.
pub struct OneRequestService<T, P> {
    config: ServiceConfig,
    _t: PhantomData<(T, P)>,
}

impl<T, P> Service for OneRequestService<T, P>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = Io<T, P>;
    type Response = (Request, Framed<T, Codec>);
    type Error = ParseError;
    type Future = OneRequestServiceResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        OneRequestServiceResponse {
            framed: Some(Framed::new(
                req.into_parts().0,
                Codec::new(self.config.clone()),
            )),
        }
    }
}

#[doc(hidden)]
pub struct OneRequestServiceResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    framed: Option<Framed<T, Codec>>,
}

impl<T> Future for OneRequestServiceResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = (Request, Framed<T, Codec>);
    type Error = ParseError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.framed.as_mut().unwrap().poll()? {
            Async::Ready(Some(req)) => match req {
                Message::Item(req) => {
                    Ok(Async::Ready((req, self.framed.take().unwrap())))
                }
                Message::Chunk(_) => unreachable!("Something is wrong"),
            },
            Async::Ready(None) => Err(ParseError::Incomplete),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

use std::fmt;
use std::marker::PhantomData;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_server_config::{Io, ServerConfig as SrvConfig};
use actix_service::{IntoNewService, NewService, Service};
use actix_utils::cloneable::CloneableService;
use futures::future::{ok, FutureResult};
use futures::{try_ready, Async, Future, IntoFuture, Poll, Stream};

use crate::body::MessageBody;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, Error, ParseError};
use crate::request::Request;
use crate::response::Response;

use super::codec::Codec;
use super::dispatcher::Dispatcher;
use super::{ExpectHandler, Message};

/// `NewService` implementation for HTTP1 transport
pub struct H1Service<T, P, S, B, X = ExpectHandler> {
    srv: S,
    cfg: ServiceConfig,
    expect: X,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> H1Service<T, P, S, B>
where
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    /// Create new `HttpService` instance with default config.
    pub fn new<F: IntoNewService<S, SrvConfig>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H1Service {
            cfg,
            srv: service.into_new_service(),
            expect: ExpectHandler,
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
            expect: ExpectHandler,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X> H1Service<T, P, S, B, X>
where
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
{
    pub fn expect<U>(self, expect: U) -> H1Service<T, P, S, B, U>
    where
        U: NewService<Request = Request, Response = Request>,
        U::Error: Into<Error>,
        U::InitError: fmt::Debug,
    {
        H1Service {
            expect,
            cfg: self.cfg,
            srv: self.srv,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X> NewService<SrvConfig> for H1Service<T, P, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type InitError = ();
    type Service = H1ServiceHandler<T, P, S::Service, B, X::Service>;
    type Future = H1ServiceResponse<T, P, S, B, X>;

    fn new_service(&self, cfg: &SrvConfig) -> Self::Future {
        H1ServiceResponse {
            fut: self.srv.new_service(cfg).into_future(),
            fut_ex: Some(self.expect.new_service(&())),
            expect: None,
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct H1ServiceResponse<T, P, S, B, X>
where
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
{
    fut: S::Future,
    fut_ex: Option<X::Future>,
    expect: Option<X::Service>,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B, X> Future for H1ServiceResponse<T, P, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    S::InitError: fmt::Debug,
    B: MessageBody,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
{
    type Item = H1ServiceHandler<T, P, S::Service, B, X::Service>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_ex {
            let expect = try_ready!(fut
                .poll()
                .map_err(|e| log::error!("Init http service error: {:?}", e)));
            self.expect = Some(expect);
            self.fut_ex.take();
        }

        let service = try_ready!(self
            .fut
            .poll()
            .map_err(|e| log::error!("Init http service error: {:?}", e)));
        Ok(Async::Ready(H1ServiceHandler::new(
            self.cfg.take().unwrap(),
            service,
            self.expect.take().unwrap(),
        )))
    }
}

/// `Service` implementation for HTTP1 transport
pub struct H1ServiceHandler<T, P, S, B, X> {
    srv: CloneableService<S>,
    expect: CloneableService<X>,
    cfg: ServiceConfig,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B, X> H1ServiceHandler<T, P, S, B, X>
where
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
{
    fn new(cfg: ServiceConfig, srv: S, expect: X) -> H1ServiceHandler<T, P, S, B, X> {
        H1ServiceHandler {
            srv: CloneableService::new(srv),
            expect: CloneableService::new(expect),
            cfg,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X> Service for H1ServiceHandler<T, P, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
{
    type Request = Io<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = Dispatcher<T, S, B, X>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        let ready = self
            .expect
            .poll_ready()
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready();

        let ready = self
            .srv
            .poll_ready()
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready()
            && ready;

        if ready {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        Dispatcher::new(
            req.into_parts().0,
            self.cfg.clone(),
            self.srv.clone(),
            self.expect.clone(),
        )
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

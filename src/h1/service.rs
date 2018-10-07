use std::fmt::{Debug, Display};
use std::marker::PhantomData;

use actix_net::codec::Framed;
use actix_net::service::{IntoNewService, NewService, Service};
use futures::{future, Async, Future, Poll, Stream};
use tokio_io::{AsyncRead, AsyncWrite};

use config::ServiceConfig;
use error::{DispatchError, ParseError};
use request::Request;
use response::Response;

use super::codec::{Codec, InMessage};
use super::dispatcher::Dispatcher;

/// `NewService` implementation for HTTP1 transport
pub struct H1Service<T, S> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<T>,
}

impl<T, S> H1Service<T, S>
where
    S: NewService,
    S::Service: Clone,
    S::Error: Debug + Display,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S>>(cfg: ServiceConfig, service: F) -> Self {
        H1Service {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }
}

impl<T, S> NewService for H1Service<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<Request = Request, Response = Response> + Clone,
    S::Service: Clone,
    S::Error: Debug + Display,
{
    type Request = T;
    type Response = ();
    type Error = DispatchError<S::Error>;
    type InitError = S::InitError;
    type Service = H1ServiceHandler<T, S::Service>;
    type Future = H1ServiceResponse<T, S>;

    fn new_service(&self) -> Self::Future {
        H1ServiceResponse {
            fut: self.srv.new_service(),
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

pub struct H1ServiceResponse<T, S: NewService> {
    fut: S::Future,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<T>,
}

impl<T, S> Future for H1ServiceResponse<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<Request = Request, Response = Response>,
    S::Service: Clone,
    S::Error: Debug + Display,
{
    type Item = H1ServiceHandler<T, S::Service>;
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
pub struct H1ServiceHandler<T, S> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<T>,
}

impl<T, S> H1ServiceHandler<T, S>
where
    S: Service<Request = Request, Response = Response> + Clone,
    S::Error: Debug + Display,
{
    fn new(cfg: ServiceConfig, srv: S) -> H1ServiceHandler<T, S> {
        H1ServiceHandler {
            srv,
            cfg,
            _t: PhantomData,
        }
    }
}

impl<T, S> Service for H1ServiceHandler<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response> + Clone,
    S::Error: Debug + Display,
{
    type Request = T;
    type Response = ();
    type Error = DispatchError<S::Error>;
    type Future = Dispatcher<T, S>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.srv.poll_ready().map_err(|e| DispatchError::Service(e))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        Dispatcher::new(req, self.cfg.clone(), self.srv.clone())
    }
}

/// `NewService` that implements, read one request from framed object feature.
pub struct TakeRequest<T> {
    _t: PhantomData<T>,
}

impl<T> TakeRequest<T> {
    /// Create new `TakeRequest` instance.
    pub fn new() -> Self {
        TakeRequest { _t: PhantomData }
    }
}

impl<T> NewService for TakeRequest<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = Framed<T, Codec>;
    type Response = (Option<InMessage>, Framed<T, Codec>);
    type Error = ParseError;
    type InitError = ();
    type Service = TakeRequestService<T>;
    type Future = future::FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        future::ok(TakeRequestService { _t: PhantomData })
    }
}

/// `NewService` that implements, read one request from framed object feature.
pub struct TakeRequestService<T> {
    _t: PhantomData<T>,
}

impl<T> Service for TakeRequestService<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = Framed<T, Codec>;
    type Response = (Option<InMessage>, Framed<T, Codec>);
    type Error = ParseError;
    type Future = TakeRequestServiceResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, framed: Self::Request) -> Self::Future {
        TakeRequestServiceResponse {
            framed: Some(framed),
        }
    }
}

pub struct TakeRequestServiceResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    framed: Option<Framed<T, Codec>>,
}

impl<T> Future for TakeRequestServiceResponse<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = (Option<InMessage>, Framed<T, Codec>);
    type Error = ParseError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.framed.as_mut().unwrap().poll()? {
            Async::Ready(item) => Ok(Async::Ready((item, self.framed.take().unwrap()))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

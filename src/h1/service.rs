use std::fmt::{Debug, Display};
use std::marker::PhantomData;

use actix_net::service::{IntoNewService, NewService, Service};
use futures::{Async, Future, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

use config::ServiceConfig;
use error::DispatchError;
use httpresponse::HttpResponse;
use request::Request;

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
    S: NewService<Request = Request, Response = HttpResponse> + Clone,
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
    S: NewService<Request = Request, Response = HttpResponse>,
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
    S: Service<Request = Request, Response = HttpResponse> + Clone,
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
    S: Service<Request = Request, Response = HttpResponse> + Clone,
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

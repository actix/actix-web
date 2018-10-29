use std::fmt::Debug;
use std::marker::PhantomData;
use std::net;

use actix_net::codec::Framed;
use actix_net::service::{IntoNewService, NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll, Stream};
use tokio_io::{AsyncRead, AsyncWrite};

use config::{KeepAlive, ServiceConfig};
use error::{DispatchError, ParseError};
use request::Request;
use response::Response;

use super::codec::Codec;
use super::dispatcher::Dispatcher;
use super::{H1ServiceResult, Message};

/// `NewService` implementation for HTTP1 transport
pub struct H1Service<T, S> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<T>,
}

impl<T, S> H1Service<T, S>
where
    S: NewService<Request = Request, Response = Response> + Clone,
    S::Service: Clone,
    S::Error: Debug,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H1Service {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }

    /// Create builder for `HttpService` instance.
    pub fn build() -> H1ServiceBuilder<T, S> {
        H1ServiceBuilder::new()
    }
}

impl<T, S> NewService for H1Service<T, S>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<Request = Request, Response = Response> + Clone,
    S::Service: Clone,
    S::Error: Debug,
{
    type Request = T;
    type Response = H1ServiceResult<T>;
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

/// A http/1 new service builder
///
/// This type can be used to construct an instance of `ServiceConfig` through a
/// builder-like pattern.
pub struct H1ServiceBuilder<T, S> {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_disconnect: u64,
    host: String,
    addr: net::SocketAddr,
    secure: bool,
    _t: PhantomData<(T, S)>,
}

impl<T, S> H1ServiceBuilder<T, S>
where
    S: NewService,
    S::Service: Clone,
    S::Error: Debug,
{
    /// Create instance of `ServiceConfigBuilder`
    pub fn new() -> H1ServiceBuilder<T, S> {
        H1ServiceBuilder {
            keep_alive: KeepAlive::Timeout(5),
            client_timeout: 5000,
            client_disconnect: 0,
            secure: false,
            host: "localhost".to_owned(),
            addr: "127.0.0.1:8080".parse().unwrap(),
            _t: PhantomData,
        }
    }

    /// Enable secure flag for current server.
    /// This flags also enables `client disconnect timeout`.
    ///
    /// By default this flag is set to false.
    pub fn secure(mut self) -> Self {
        self.secure = true;
        if self.client_disconnect == 0 {
            self.client_disconnect = 3000;
        }
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<U: Into<KeepAlive>>(mut self, val: U) -> Self {
        self.keep_alive = val.into();
        self
    }

    /// Set server client timeout in milliseconds for first request.
    ///
    /// Defines a timeout for reading client request header. If a client does not transmit
    /// the entire set headers within this time, the request is terminated with
    /// the 408 (Request Time-out) error.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default client timeout is set to 5000 milliseconds.
    pub fn client_timeout(mut self, val: u64) -> Self {
        self.client_timeout = val;
        self
    }

    /// Set server connection disconnect timeout in milliseconds.
    ///
    /// Defines a timeout for disconnect connection. If a disconnect procedure does not complete
    /// within this time, the request get dropped. This timeout affects secure connections.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default disconnect timeout is set to 3000 milliseconds.
    pub fn client_disconnect(mut self, val: u64) -> Self {
        self.client_disconnect = val;
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn server_hostname(mut self, val: &str) -> Self {
        self.host = val.to_owned();
        self
    }

    /// Set server ip address.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default server address is set to a "127.0.0.1:8080"
    pub fn server_address<U: net::ToSocketAddrs>(mut self, addr: U) -> Self {
        match addr.to_socket_addrs() {
            Err(err) => error!("Can not convert to SocketAddr: {}", err),
            Ok(mut addrs) => if let Some(addr) = addrs.next() {
                self.addr = addr;
            },
        }
        self
    }

    /// Finish service configuration and create `H1Service` instance.
    pub fn finish<F: IntoNewService<S>>(self, service: F) -> H1Service<T, S> {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
        );
        H1Service {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
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
    S::Error: Debug,
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
    S::Error: Debug,
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
    S::Error: Debug,
{
    type Request = T;
    type Response = H1ServiceResult<T>;
    type Error = DispatchError<S::Error>;
    type Future = Dispatcher<T, S>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.srv.poll_ready().map_err(DispatchError::Service)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        Dispatcher::new(req, self.cfg.clone(), self.srv.clone())
    }
}

/// `NewService` implementation for `OneRequestService` service
pub struct OneRequest<T> {
    config: ServiceConfig,
    _t: PhantomData<T>,
}

impl<T> OneRequest<T>
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

impl<T> NewService for OneRequest<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = T;
    type Response = (Request, Framed<T, Codec>);
    type Error = ParseError;
    type InitError = ();
    type Service = OneRequestService<T>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(OneRequestService {
            config: self.config.clone(),
            _t: PhantomData,
        })
    }
}

/// `Service` implementation for HTTP1 transport. Reads one request and returns
/// request and framed object.
pub struct OneRequestService<T> {
    config: ServiceConfig,
    _t: PhantomData<T>,
}

impl<T> Service for OneRequestService<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Request = T;
    type Response = (Request, Framed<T, Codec>);
    type Error = ParseError;
    type Future = OneRequestServiceResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        OneRequestServiceResponse {
            framed: Some(Framed::new(req, Codec::new(self.config.clone()))),
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

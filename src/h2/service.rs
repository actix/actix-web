use std::fmt::Debug;
use std::marker::PhantomData;
use std::net;

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_service::{IntoNewService, NewService, Service};
use bytes::Bytes;
use futures::future::{ok, FutureResult};
use futures::{try_ready, Async, Future, Poll, Stream};
use h2::server::{self, Connection, Handshake};
use log::error;

use crate::body::MessageBody;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, ParseError};
use crate::request::Request;
use crate::response::Response;

// use super::dispatcher::Dispatcher;
use super::H2ServiceResult;

/// `NewService` implementation for HTTP2 transport
pub struct H2Service<T, S, B> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> H2Service<T, S, B>
where
    S: NewService<Request = Request, Response = Response<B>> + Clone,
    S::Service: Clone,
    S::Error: Debug,
    B: MessageBody,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        H2Service {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }

    /// Create builder for `HttpService` instance.
    pub fn build() -> H2ServiceBuilder<T, S> {
        H2ServiceBuilder::new()
    }
}

impl<T, S, B> NewService for H2Service<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<Request = Request, Response = Response<B>> + Clone,
    S::Service: Clone,
    S::Error: Debug,
    B: MessageBody,
{
    type Request = T;
    type Response = H2ServiceResult<T>;
    type Error = (); //DispatchError<S::Error>;
    type InitError = S::InitError;
    type Service = H2ServiceHandler<T, S::Service, B>;
    type Future = H2ServiceResponse<T, S, B>;

    fn new_service(&self) -> Self::Future {
        H2ServiceResponse {
            fut: self.srv.new_service(),
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

/// A http/2 new service builder
///
/// This type can be used to construct an instance of `ServiceConfig` through a
/// builder-like pattern.
pub struct H2ServiceBuilder<T, S> {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_disconnect: u64,
    host: String,
    addr: net::SocketAddr,
    secure: bool,
    _t: PhantomData<(T, S)>,
}

impl<T, S> H2ServiceBuilder<T, S>
where
    S: NewService<Request = Request>,
    S::Service: Clone,
    S::Error: Debug,
{
    /// Create instance of `H2ServiceBuilder`
    pub fn new() -> H2ServiceBuilder<T, S> {
        H2ServiceBuilder {
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
            Ok(mut addrs) => {
                if let Some(addr) = addrs.next() {
                    self.addr = addr;
                }
            }
        }
        self
    }

    /// Finish service configuration and create `H1Service` instance.
    pub fn finish<F, B>(self, service: F) -> H2Service<T, S, B>
    where
        B: MessageBody,
        F: IntoNewService<S>,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
        );
        H2Service {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct H2ServiceResponse<T, S: NewService, B> {
    fut: S::Future,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> Future for H2ServiceResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<Request = Request, Response = Response<B>>,
    S::Service: Clone,
    S::Error: Debug,
    B: MessageBody,
{
    type Item = H2ServiceHandler<T, S::Service, B>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let service = try_ready!(self.fut.poll());
        Ok(Async::Ready(H2ServiceHandler::new(
            self.cfg.take().unwrap(),
            service,
        )))
    }
}

/// `Service` implementation for http/2 transport
pub struct H2ServiceHandler<T, S, B> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> H2ServiceHandler<T, S, B>
where
    S: Service<Request = Request, Response = Response<B>> + Clone,
    S::Error: Debug,
    B: MessageBody,
{
    fn new(cfg: ServiceConfig, srv: S) -> H2ServiceHandler<T, S, B> {
        H2ServiceHandler {
            srv,
            cfg,
            _t: PhantomData,
        }
    }
}

impl<T, S, B> Service for H2ServiceHandler<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response<B>> + Clone,
    S::Error: Debug,
    B: MessageBody,
{
    type Request = T;
    type Response = H2ServiceResult<T>;
    type Error = (); // DispatchError<S::Error>;
    type Future = H2ServiceHandlerResponse<T, S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.srv.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, req: T) -> Self::Future {
        H2ServiceHandlerResponse {
            state: State::Handshake(server::handshake(req)),
            _t: PhantomData,
        }
    }
}

enum State<T: AsyncRead + AsyncWrite> {
    Handshake(Handshake<T, Bytes>),
    Connection(Connection<T, Bytes>),
    Empty,
}

pub struct H2ServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response<B>> + Clone,
    S::Error: Debug,
    B: MessageBody,
{
    state: State<T>,
    _t: PhantomData<S>,
}

impl<T, S, B> Future for H2ServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request, Response = Response<B>> + Clone,
    S::Error: Debug,
    B: MessageBody,
{
    type Item = H2ServiceResult<T>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        unimplemented!()
    }
}

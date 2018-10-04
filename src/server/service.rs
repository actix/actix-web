use std::marker::PhantomData;
use std::time::Duration;

use actix_net::service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Poll};

use super::channel::{H1Channel, HttpChannel};
use super::error::HttpDispatchError;
use super::handler::HttpHandler;
use super::settings::ServiceConfig;
use super::IoStream;
use error::Error;

/// `NewService` implementation for HTTP1/HTTP2 transports
pub struct HttpService<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    settings: ServiceConfig<H>,
    _t: PhantomData<Io>,
}

impl<H, Io> HttpService<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    /// Create new `HttpService` instance.
    pub fn new(settings: ServiceConfig<H>) -> Self {
        HttpService {
            settings,
            _t: PhantomData,
        }
    }
}

impl<H, Io> NewService for HttpService<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    type Request = Io;
    type Response = ();
    type Error = HttpDispatchError<Error>;
    type InitError = ();
    type Service = HttpServiceHandler<H, Io>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(HttpServiceHandler::new(self.settings.clone()))
    }
}

pub struct HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    settings: ServiceConfig<H>,
    _t: PhantomData<Io>,
}

impl<H, Io> HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    fn new(settings: ServiceConfig<H>) -> HttpServiceHandler<H, Io> {
        HttpServiceHandler {
            settings,
            _t: PhantomData,
        }
    }
}

impl<H, Io> Service for HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    type Request = Io;
    type Response = ();
    type Error = HttpDispatchError<Error>;
    type Future = HttpChannel<Io, H>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        HttpChannel::new(self.settings.clone(), req)
    }
}

/// `NewService` implementation for HTTP1 transport
pub struct H1Service<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    settings: ServiceConfig<H>,
    _t: PhantomData<Io>,
}

impl<H, Io> H1Service<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    /// Create new `HttpService` instance.
    pub fn new(settings: ServiceConfig<H>) -> Self {
        H1Service {
            settings,
            _t: PhantomData,
        }
    }
}

impl<H, Io> NewService for H1Service<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    type Request = Io;
    type Response = ();
    type Error = HttpDispatchError<Error>;
    type InitError = ();
    type Service = H1ServiceHandler<H, Io>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(H1ServiceHandler::new(self.settings.clone()))
    }
}

/// `Service` implementation for HTTP1 transport
pub struct H1ServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    settings: ServiceConfig<H>,
    _t: PhantomData<Io>,
}

impl<H, Io> H1ServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    fn new(settings: ServiceConfig<H>) -> H1ServiceHandler<H, Io> {
        H1ServiceHandler {
            settings,
            _t: PhantomData,
        }
    }
}

impl<H, Io> Service for H1ServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    type Request = Io;
    type Response = ();
    type Error = HttpDispatchError<Error>;
    type Future = H1Channel<Io, H>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        H1Channel::new(self.settings.clone(), req)
    }
}

/// `NewService` implementation for stream configuration service
///
/// Stream configuration service allows to change some socket level
/// parameters. for example `tcp nodelay` or `tcp keep-alive`.
pub struct StreamConfiguration<T, E> {
    no_delay: Option<bool>,
    tcp_ka: Option<Option<Duration>>,
    _t: PhantomData<(T, E)>,
}

impl<T, E> Default for StreamConfiguration<T, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, E> StreamConfiguration<T, E> {
    /// Create new `StreamConfigurationService` instance.
    pub fn new() -> Self {
        Self {
            no_delay: None,
            tcp_ka: None,
            _t: PhantomData,
        }
    }

    /// Sets the value of the `TCP_NODELAY` option on this socket.
    pub fn nodelay(mut self, nodelay: bool) -> Self {
        self.no_delay = Some(nodelay);
        self
    }

    /// Sets whether keepalive messages are enabled to be sent on this socket.
    pub fn tcp_keepalive(mut self, keepalive: Option<Duration>) -> Self {
        self.tcp_ka = Some(keepalive);
        self
    }
}

impl<T: IoStream, E> NewService for StreamConfiguration<T, E> {
    type Request = T;
    type Response = T;
    type Error = E;
    type InitError = ();
    type Service = StreamConfigurationService<T, E>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(StreamConfigurationService {
            no_delay: self.no_delay,
            tcp_ka: self.tcp_ka,
            _t: PhantomData,
        })
    }
}

/// Stream configuration service
///
/// Stream configuration service allows to change some socket level
/// parameters. for example `tcp nodelay` or `tcp keep-alive`.
pub struct StreamConfigurationService<T, E> {
    no_delay: Option<bool>,
    tcp_ka: Option<Option<Duration>>,
    _t: PhantomData<(T, E)>,
}

impl<T, E> Service for StreamConfigurationService<T, E>
where
    T: IoStream,
{
    type Request = T;
    type Response = T;
    type Error = E;
    type Future = FutureResult<T, E>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, mut req: Self::Request) -> Self::Future {
        if let Some(no_delay) = self.no_delay {
            if req.set_nodelay(no_delay).is_err() {
                error!("Can not set socket no-delay option");
            }
        }
        if let Some(keepalive) = self.tcp_ka {
            if req.set_keepalive(keepalive).is_err() {
                error!("Can not set socket keep-alive option");
            }
        }

        ok(req)
    }
}

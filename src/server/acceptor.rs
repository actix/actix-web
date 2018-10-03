use std::time::Duration;
use std::{fmt, net};

use actix_net::server::ServerMessage;
use actix_net::service::{NewService, Service};
use futures::future::{err, ok, Either, FutureResult};
use futures::{Async, Future, Poll};
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;
use tokio_timer::{sleep, Delay};

use super::channel::HttpProtocol;
use super::error::AcceptorError;
use super::handler::HttpHandler;
use super::settings::ServiceConfig;
use super::IoStream;

/// This trait indicates types that can create acceptor service for http server.
pub trait AcceptorServiceFactory: Send + Clone + 'static {
    type Io: IoStream + Send;
    type NewService: NewService<Request = TcpStream, Response = Self::Io>;

    fn create(&self) -> Self::NewService;
}

impl<F, T> AcceptorServiceFactory for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T::Response: IoStream + Send,
    T: NewService<Request = TcpStream>,
    T::InitError: fmt::Debug,
{
    type Io = T::Response;
    type NewService = T;

    fn create(&self) -> T {
        (self)()
    }
}

#[derive(Clone)]
/// Default acceptor service convert `TcpStream` to a `tokio_tcp::TcpStream`
pub(crate) struct DefaultAcceptor;

impl AcceptorServiceFactory for DefaultAcceptor {
    type Io = TcpStream;
    type NewService = DefaultAcceptor;

    fn create(&self) -> Self::NewService {
        DefaultAcceptor
    }
}

impl NewService for DefaultAcceptor {
    type Request = TcpStream;
    type Response = TcpStream;
    type Error = ();
    type InitError = ();
    type Service = DefaultAcceptor;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(DefaultAcceptor)
    }
}

impl Service for DefaultAcceptor {
    type Request = TcpStream;
    type Response = TcpStream;
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        ok(req)
    }
}

pub(crate) struct TcpAcceptor<T> {
    inner: T,
}

impl<T, E> TcpAcceptor<T>
where
    T: NewService<Request = TcpStream, Error = AcceptorError<E>>,
    T::InitError: fmt::Debug,
{
    pub(crate) fn new(inner: T) -> Self {
        TcpAcceptor { inner }
    }
}

impl<T, E> NewService for TcpAcceptor<T>
where
    T: NewService<Request = TcpStream, Error = AcceptorError<E>>,
    T::InitError: fmt::Debug,
{
    type Request = net::TcpStream;
    type Response = T::Response;
    type Error = AcceptorError<E>;
    type InitError = T::InitError;
    type Service = TcpAcceptorService<T::Service>;
    type Future = TcpAcceptorResponse<T>;

    fn new_service(&self) -> Self::Future {
        TcpAcceptorResponse {
            fut: self.inner.new_service(),
        }
    }
}

pub(crate) struct TcpAcceptorResponse<T>
where
    T: NewService<Request = TcpStream>,
    T::InitError: fmt::Debug,
{
    fut: T::Future,
}

impl<T> Future for TcpAcceptorResponse<T>
where
    T: NewService<Request = TcpStream>,
    T::InitError: fmt::Debug,
{
    type Item = TcpAcceptorService<T::Service>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(service)) => {
                Ok(Async::Ready(TcpAcceptorService { inner: service }))
            }
            Err(e) => {
                error!("Can not create accetor service: {:?}", e);
                Err(e)
            }
        }
    }
}

pub(crate) struct TcpAcceptorService<T> {
    inner: T,
}

impl<T, E> Service for TcpAcceptorService<T>
where
    T: Service<Request = TcpStream, Error = AcceptorError<E>>,
{
    type Request = net::TcpStream;
    type Response = T::Response;
    type Error = AcceptorError<E>;
    type Future = Either<T::Future, FutureResult<Self::Response, Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let stream = TcpStream::from_std(req, &Handle::default()).map_err(|e| {
            error!("Can not convert to an async tcp stream: {}", e);
            AcceptorError::Io(e)
        });

        match stream {
            Ok(stream) => Either::A(self.inner.call(stream)),
            Err(e) => Either::B(err(e)),
        }
    }
}

#[doc(hidden)]
/// Acceptor timeout middleware
///
/// Applies timeout to request prcoessing.
pub struct AcceptorTimeout<T> {
    inner: T,
    timeout: Duration,
}

impl<T: NewService> AcceptorTimeout<T> {
    pub(crate) fn new(timeout: u64, inner: T) -> Self {
        Self {
            inner,
            timeout: Duration::from_millis(timeout),
        }
    }
}

impl<T: NewService> NewService for AcceptorTimeout<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = AcceptorError<T::Error>;
    type InitError = T::InitError;
    type Service = AcceptorTimeoutService<T::Service>;
    type Future = AcceptorTimeoutFut<T>;

    fn new_service(&self) -> Self::Future {
        AcceptorTimeoutFut {
            fut: self.inner.new_service(),
            timeout: self.timeout,
        }
    }
}

#[doc(hidden)]
pub struct AcceptorTimeoutFut<T: NewService> {
    fut: T::Future,
    timeout: Duration,
}

impl<T: NewService> Future for AcceptorTimeoutFut<T> {
    type Item = AcceptorTimeoutService<T::Service>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let inner = try_ready!(self.fut.poll());
        Ok(Async::Ready(AcceptorTimeoutService {
            inner,
            timeout: self.timeout,
        }))
    }
}

#[doc(hidden)]
/// Acceptor timeout service
///
/// Applies timeout to request prcoessing.
pub struct AcceptorTimeoutService<T> {
    inner: T,
    timeout: Duration,
}

impl<T: Service> Service for AcceptorTimeoutService<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = AcceptorError<T::Error>;
    type Future = AcceptorTimeoutResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready().map_err(AcceptorError::Service)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        AcceptorTimeoutResponse {
            fut: self.inner.call(req),
            sleep: sleep(self.timeout),
        }
    }
}

#[doc(hidden)]
pub struct AcceptorTimeoutResponse<T: Service> {
    fut: T::Future,
    sleep: Delay,
}

impl<T: Service> Future for AcceptorTimeoutResponse<T> {
    type Item = T::Response;
    type Error = AcceptorError<T::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll().map_err(AcceptorError::Service)? {
            Async::NotReady => match self.sleep.poll() {
                Err(_) => Err(AcceptorError::Timeout),
                Ok(Async::Ready(_)) => Err(AcceptorError::Timeout),
                Ok(Async::NotReady) => Ok(Async::NotReady),
            },
            Async::Ready(resp) => Ok(Async::Ready(resp)),
        }
    }
}

pub(crate) struct ServerMessageAcceptor<T, H: HttpHandler> {
    inner: T,
    settings: ServiceConfig<H>,
}

impl<T, H> ServerMessageAcceptor<T, H>
where
    H: HttpHandler,
    T: NewService<Request = net::TcpStream>,
{
    pub(crate) fn new(settings: ServiceConfig<H>, inner: T) -> Self {
        ServerMessageAcceptor { inner, settings }
    }
}

impl<T, H> NewService for ServerMessageAcceptor<T, H>
where
    H: HttpHandler,
    T: NewService<Request = net::TcpStream>,
{
    type Request = ServerMessage;
    type Response = ();
    type Error = T::Error;
    type InitError = T::InitError;
    type Service = ServerMessageAcceptorService<T::Service, H>;
    type Future = ServerMessageAcceptorResponse<T, H>;

    fn new_service(&self) -> Self::Future {
        ServerMessageAcceptorResponse {
            fut: self.inner.new_service(),
            settings: self.settings.clone(),
        }
    }
}

pub(crate) struct ServerMessageAcceptorResponse<T, H>
where
    H: HttpHandler,
    T: NewService<Request = net::TcpStream>,
{
    fut: T::Future,
    settings: ServiceConfig<H>,
}

impl<T, H> Future for ServerMessageAcceptorResponse<T, H>
where
    H: HttpHandler,
    T: NewService<Request = net::TcpStream>,
{
    type Item = ServerMessageAcceptorService<T::Service, H>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(service) => Ok(Async::Ready(ServerMessageAcceptorService {
                inner: service,
                settings: self.settings.clone(),
            })),
        }
    }
}

pub(crate) struct ServerMessageAcceptorService<T, H: HttpHandler> {
    inner: T,
    settings: ServiceConfig<H>,
}

impl<T, H> Service for ServerMessageAcceptorService<T, H>
where
    H: HttpHandler,
    T: Service<Request = net::TcpStream>,
{
    type Request = ServerMessage;
    type Response = ();
    type Error = T::Error;
    type Future =
        Either<ServerMessageAcceptorServiceFut<T>, FutureResult<(), Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        match req {
            ServerMessage::Connect(stream) => {
                Either::A(ServerMessageAcceptorServiceFut {
                    fut: self.inner.call(stream),
                })
            }
            ServerMessage::Shutdown(_) => Either::B(ok(())),
            ServerMessage::ForceShutdown => {
                self.settings
                    .head()
                    .traverse(|proto: &mut HttpProtocol<TcpStream, H>| proto.shutdown());
                Either::B(ok(()))
            }
        }
    }
}

pub(crate) struct ServerMessageAcceptorServiceFut<T: Service> {
    fut: T::Future,
}

impl<T> Future for ServerMessageAcceptorServiceFut<T>
where
    T: Service,
{
    type Item = ();
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(_) => Ok(Async::Ready(())),
        }
    }
}

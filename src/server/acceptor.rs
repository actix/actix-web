use std::time::Duration;

use actix_net::server::ServerMessage;
use actix_net::service::{NewService, Service};
use futures::future::{err, ok, Either, FutureResult};
use futures::{Async, Future, Poll};
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;
use tokio_timer::{sleep, Delay};

use super::handler::HttpHandler;
use super::settings::WorkerSettings;
use super::IoStream;

/// This trait indicates types that can create acceptor service for http server.
pub trait AcceptorServiceFactory: Send + Clone + 'static {
    type Io: IoStream + Send;
    type NewService: NewService<
        Request = TcpStream,
        Response = Self::Io,
        Error = (),
        InitError = (),
    >;

    fn create(&self) -> Self::NewService;
}

impl<F, T> AcceptorServiceFactory for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T::Response: IoStream + Send,
    T: NewService<Request = TcpStream, Error = (), InitError = ()>,
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

pub(crate) struct TcpAcceptor<T, H: HttpHandler> {
    inner: T,
    settings: WorkerSettings<H>,
}

impl<T, H> TcpAcceptor<T, H>
where
    H: HttpHandler,
    T: NewService<Request = TcpStream>,
{
    pub(crate) fn new(settings: WorkerSettings<H>, inner: T) -> Self {
        TcpAcceptor { inner, settings }
    }
}

impl<T, H> NewService for TcpAcceptor<T, H>
where
    H: HttpHandler,
    T: NewService<Request = TcpStream>,
{
    type Request = ServerMessage;
    type Response = ();
    type Error = ();
    type InitError = ();
    type Service = TcpAcceptorService<T::Service, H>;
    type Future = TcpAcceptorResponse<T, H>;

    fn new_service(&self) -> Self::Future {
        TcpAcceptorResponse {
            fut: self.inner.new_service(),
            settings: self.settings.clone(),
        }
    }
}

pub(crate) struct TcpAcceptorResponse<T, H>
where
    H: HttpHandler,
    T: NewService<Request = TcpStream>,
{
    fut: T::Future,
    settings: WorkerSettings<H>,
}

impl<T, H> Future for TcpAcceptorResponse<T, H>
where
    H: HttpHandler,
    T: NewService<Request = TcpStream>,
{
    type Item = TcpAcceptorService<T::Service, H>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Err(_) => Err(()),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(service)) => Ok(Async::Ready(TcpAcceptorService {
                inner: service,
                settings: self.settings.clone(),
            })),
        }
    }
}

pub(crate) struct TcpAcceptorService<T, H: HttpHandler> {
    inner: T,
    settings: WorkerSettings<H>,
}

impl<T, H> Service for TcpAcceptorService<T, H>
where
    H: HttpHandler,
    T: Service<Request = TcpStream>,
{
    type Request = ServerMessage;
    type Response = ();
    type Error = ();
    type Future = Either<TcpAcceptorServiceFut<T::Future>, FutureResult<(), ()>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        match req {
            ServerMessage::Connect(stream) => {
                let stream =
                    TcpStream::from_std(stream, &Handle::default()).map_err(|e| {
                        error!("Can not convert to an async tcp stream: {}", e);
                    });

                if let Ok(stream) = stream {
                    Either::A(TcpAcceptorServiceFut {
                        fut: self.inner.call(stream),
                    })
                } else {
                    Either::B(err(()))
                }
            }
            ServerMessage::Shutdown(timeout) => Either::B(ok(())),
            ServerMessage::ForceShutdown => {
                // self.settings.head().traverse::<TcpStream, H>();
                Either::B(ok(()))
            }
        }
    }
}

pub(crate) struct TcpAcceptorServiceFut<T: Future> {
    fut: T,
}

impl<T> Future for TcpAcceptorServiceFut<T>
where
    T: Future,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Err(_) => Err(()),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(_)) => Ok(Async::Ready(())),
        }
    }
}

/// Errors produced by `AcceptorTimeout` service.
#[derive(Debug)]
pub enum TimeoutError<T> {
    /// The inner service error
    Service(T),

    /// The request did not complete within the specified timeout.
    Timeout,
}

/// Acceptor timeout middleware
///
/// Applies timeout to request prcoessing.
pub(crate) struct AcceptorTimeout<T> {
    inner: T,
    timeout: usize,
}

impl<T: NewService> AcceptorTimeout<T> {
    pub(crate) fn new(timeout: usize, inner: T) -> Self {
        Self { inner, timeout }
    }
}

impl<T: NewService> NewService for AcceptorTimeout<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = TimeoutError<T::Error>;
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
pub(crate) struct AcceptorTimeoutFut<T: NewService> {
    fut: T::Future,
    timeout: usize,
}

impl<T: NewService> Future for AcceptorTimeoutFut<T> {
    type Item = AcceptorTimeoutService<T::Service>;
    type Error = T::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let inner = try_ready!(self.fut.poll());
        Ok(Async::Ready(AcceptorTimeoutService {
            inner,
            timeout: self.timeout as u64,
        }))
    }
}

/// Acceptor timeout service
///
/// Applies timeout to request prcoessing.
pub(crate) struct AcceptorTimeoutService<T> {
    inner: T,
    timeout: u64,
}

impl<T: Service> Service for AcceptorTimeoutService<T> {
    type Request = T::Request;
    type Response = T::Response;
    type Error = TimeoutError<T::Error>;
    type Future = AcceptorTimeoutResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready().map_err(TimeoutError::Service)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        AcceptorTimeoutResponse {
            fut: self.inner.call(req),
            sleep: sleep(Duration::from_millis(self.timeout)),
        }
    }
}

pub(crate) struct AcceptorTimeoutResponse<T: Service> {
    fut: T::Future,
    sleep: Delay,
}
impl<T: Service> Future for AcceptorTimeoutResponse<T> {
    type Item = T::Response;
    type Error = TimeoutError<T::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::NotReady) => match self.sleep.poll() {
                Err(_) => Err(TimeoutError::Timeout),
                Ok(Async::Ready(_)) => Err(TimeoutError::Timeout),
                Ok(Async::NotReady) => Ok(Async::NotReady),
            },
            Ok(Async::Ready(resp)) => Ok(Async::Ready(resp)),
            Err(err) => Err(TimeoutError::Service(err)),
        }
    }
}

use std::marker::PhantomData;
use std::net;

use actix_net::server;
use actix_net::service::{NewService, NewServiceExt, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Poll};
use tokio_tcp::TcpStream;

use super::handler::IntoHttpHandler;
use super::service::HttpService;
use super::{IoStream, KeepAlive};

pub(crate) trait ServiceFactory<H>
where
    H: IntoHttpHandler,
{
    fn register(&self, server: server::Server, lst: net::TcpListener) -> server::Server;
}

pub struct HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone,
{
    factory: F,
    acceptor: A,
    pipeline: P,
}

impl<F, H, A, P> HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone,
    H: IntoHttpHandler,
    A: AcceptorServiceFactory,
    P: HttpPipelineFactory<Io = A::Io>,
{
    pub fn new(factory: F, acceptor: A, pipeline: P) -> Self {
        Self {
            factory,
            pipeline,
            acceptor,
        }
    }

    pub fn acceptor<A1>(self, acceptor: A1) -> HttpServiceBuilder<F, H, A1, P>
    where
        A1: AcceptorServiceFactory,
    {
        HttpServiceBuilder {
            acceptor,
            pipeline: self.pipeline,
            factory: self.factory.clone(),
        }
    }

    pub fn pipeline<P1>(self, pipeline: P1) -> HttpServiceBuilder<F, H, A, P1>
    where
        P1: HttpPipelineFactory,
    {
        HttpServiceBuilder {
            pipeline,
            acceptor: self.acceptor,
            factory: self.factory.clone(),
        }
    }

    fn finish(&self) -> impl server::StreamServiceFactory {
        let pipeline = self.pipeline.clone();
        let acceptor = self.acceptor.clone();
        move || acceptor.create().and_then(pipeline.create())
    }
}

impl<F, H, A, P> Clone for HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone,
    A: AcceptorServiceFactory,
    P: HttpPipelineFactory<Io = A::Io>,
{
    fn clone(&self) -> Self {
        HttpServiceBuilder {
            factory: self.factory.clone(),
            acceptor: self.acceptor.clone(),
            pipeline: self.pipeline.clone(),
        }
    }
}

impl<F, H, A, P> ServiceFactory<H> for HttpServiceBuilder<F, H, A, P>
where
    F: Fn() -> H + Send + Clone,
    A: AcceptorServiceFactory,
    P: HttpPipelineFactory<Io = A::Io>,
    H: IntoHttpHandler,
{
    fn register(&self, server: server::Server, lst: net::TcpListener) -> server::Server {
        server.listen("actix-web", lst, self.finish())
    }
}

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

pub trait HttpPipelineFactory: Send + Clone + 'static {
    type Io: IoStream;
    type NewService: NewService<
        Request = Self::Io,
        Response = (),
        Error = (),
        InitError = (),
    >;

    fn create(&self) -> Self::NewService;
}

impl<F, T> HttpPipelineFactory for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Response = (), Error = (), InitError = ()>,
    T::Request: IoStream,
{
    type Io = T::Request;
    type NewService = T;

    fn create(&self) -> T {
        (self)()
    }
}

pub(crate) struct DefaultPipelineFactory<F, H, Io>
where
    F: Fn() -> H + Send + Clone,
{
    factory: F,
    host: Option<String>,
    addr: net::SocketAddr,
    keep_alive: KeepAlive,
    _t: PhantomData<Io>,
}

impl<F, H, Io> DefaultPipelineFactory<F, H, Io>
where
    Io: IoStream + Send,
    F: Fn() -> H + Send + Clone + 'static,
    H: IntoHttpHandler + 'static,
{
    pub fn new(
        factory: F, host: Option<String>, addr: net::SocketAddr, keep_alive: KeepAlive,
    ) -> Self {
        Self {
            factory,
            addr,
            keep_alive,
            host,
            _t: PhantomData,
        }
    }
}

impl<F, H, Io> Clone for DefaultPipelineFactory<F, H, Io>
where
    Io: IoStream,
    F: Fn() -> H + Send + Clone,
    H: IntoHttpHandler,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            addr: self.addr,
            keep_alive: self.keep_alive,
            host: self.host.clone(),
            _t: PhantomData,
        }
    }
}

impl<F, H, Io> HttpPipelineFactory for DefaultPipelineFactory<F, H, Io>
where
    Io: IoStream + Send,
    F: Fn() -> H + Send + Clone + 'static,
    H: IntoHttpHandler + 'static,
{
    type Io = Io;
    type NewService = HttpService<F, H, Io>;

    fn create(&self) -> Self::NewService {
        HttpService::new(
            self.factory.clone(),
            self.addr,
            self.host.clone(),
            self.keep_alive,
        )
    }
}

#[derive(Clone)]
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

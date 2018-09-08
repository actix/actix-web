use std::net;

use futures::future::{err, ok};
use futures::{Future, Poll};
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;

use super::{NewService, Service};

pub enum ServerMessage {
    Connect(net::TcpStream),
    Shutdown,
    ForceShutdown,
}

pub(crate) type BoxedServerService = Box<
    Service<
        Request = ServerMessage,
        Response = (),
        Error = (),
        Future = Box<Future<Item = (), Error = ()>>,
    >,
>;

pub(crate) struct ServerService<T> {
    service: T,
}

impl<T> ServerService<T> {
    fn new(service: T) -> Self {
        ServerService { service }
    }
}

impl<T> Service for ServerService<T>
where
    T: Service<Request = TcpStream, Response = (), Error = ()>,
    T::Future: 'static,
    T::Error: 'static,
{
    type Request = ServerMessage;
    type Response = ();
    type Error = ();
    type Future = Box<Future<Item = (), Error = ()>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, req: ServerMessage) -> Self::Future {
        match req {
            ServerMessage::Connect(stream) => {
                let stream = TcpStream::from_std(stream, &Handle::default()).map_err(|e| {
                    error!("Can not convert to an async tcp stream: {}", e);
                });

                if let Ok(stream) = stream {
                    Box::new(self.service.call(stream))
                } else {
                    Box::new(err(()))
                }
            }
            _ => Box::new(ok(())),
        }
    }
}

pub(crate) struct ServerNewService<F, T>
where
    F: Fn() -> T + Send + Clone,
{
    inner: F,
}

impl<F, T> ServerNewService<F, T>
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = TcpStream, Response = (), Error = (), InitError = ()> + 'static,
    T::Service: 'static,
    T::Future: 'static,
{
    pub(crate) fn create(inner: F) -> Box<ServerServiceFactory + Send> {
        Box::new(Self { inner })
    }
}

pub trait ServerServiceFactory {
    fn clone_factory(&self) -> Box<ServerServiceFactory + Send>;

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>>;
}

impl<F, T> ServerServiceFactory for ServerNewService<F, T>
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = TcpStream, Response = (), Error = (), InitError = ()> + 'static,
    T::Service: 'static,
    T::Future: 'static,
{
    fn clone_factory(&self) -> Box<ServerServiceFactory + Send> {
        Box::new(Self {
            inner: self.inner.clone(),
        })
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        Box::new((self.inner)().new_service().map(move |inner| {
            let service: BoxedServerService = Box::new(ServerService::new(inner));
            service
        }))
    }
}

impl ServerServiceFactory for Box<ServerServiceFactory> {
    fn clone_factory(&self) -> Box<ServerServiceFactory + Send> {
        self.as_ref().clone_factory()
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        self.as_ref().create()
    }
}

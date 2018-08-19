use std::{fmt, io, net};

use futures::{future, Future, Poll};
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;
use tower_service::{NewService, Service};

pub(crate) type BoxedServerService = Box<
    Service<
        Request = net::TcpStream,
        Response = (),
        Error = (),
        Future = Box<Future<Item = (), Error = ()>>,
    >,
>;

pub(crate) struct ServerService<T> {
    inner: T,
}

impl<T> Service for ServerService<T>
where
    T: Service<Request = TcpStream, Response = ()>,
    T::Future: 'static,
    T::Error: fmt::Display + 'static,
{
    type Request = net::TcpStream;
    type Response = ();
    type Error = ();
    type Future = Box<Future<Item = (), Error = ()>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready().map_err(|_| ())
    }

    fn call(&mut self, stream: net::TcpStream) -> Self::Future {
        let stream = TcpStream::from_std(stream, &Handle::default()).map_err(|e| {
            error!("Can not convert to an async tcp stream: {}", e);
        });

        if let Ok(stream) = stream {
            Box::new(self.inner.call(stream).map_err(|_| ()))
        } else {
            Box::new(future::err(()))
        }
    }
}

pub(crate) struct ServerNewService<T> {
    inner: T,
}

impl<T> ServerNewService<T>
where
    T: NewService<Request = TcpStream, Response = (), InitError = io::Error>
        + Clone
        + Send
        + 'static,
    T::Service: 'static,
    T::Future: 'static,
    T::Error: fmt::Display,
{
    pub(crate) fn create(inner: T) -> Box<ServerServiceFactory + Send> {
        Box::new(Self { inner })
    }
}

pub trait ServerServiceFactory {
    fn clone_factory(&self) -> Box<ServerServiceFactory + Send>;

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>>;
}

impl<T> ServerServiceFactory for ServerNewService<T>
where
    T: NewService<Request = TcpStream, Response = (), InitError = io::Error>
        + Clone
        + Send
        + 'static,
    T::Service: 'static,
    T::Future: 'static,
    T::Error: fmt::Display,
{
    fn clone_factory(&self) -> Box<ServerServiceFactory + Send> {
        Box::new(Self {
            inner: self.inner.clone(),
        })
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        Box::new(self.inner.new_service().map_err(|_| ()).map(|inner| {
            let service: BoxedServerService = Box::new(ServerService { inner });
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

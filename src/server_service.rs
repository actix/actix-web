use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::{fmt, io, net};

use futures::{future, Future, Poll};
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;

use super::{Config, NewService, Service};

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
    counter: Arc<AtomicUsize>,
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
            let counter = self.counter.clone();
            let _ = counter.fetch_add(1, Ordering::Relaxed);
            Box::new(self.inner.call(stream).map_err(|_| ()).map(move |_| {
                let _ = counter.fetch_sub(1, Ordering::Relaxed);
            }))
        } else {
            Box::new(future::err(()))
        }
    }
}

pub(crate) struct ServerNewService<F, T, C> where F: Fn() -> T + Send + Clone {
    inner: F,
    config: C,
    counter: Arc<AtomicUsize>,
}

impl<F, T, C: Config> ServerNewService<F, T, C>
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = TcpStream, Response = (), Config = C, InitError = io::Error> + 'static,
    T::Service: 'static,
    T::Future: 'static,
    T::Error: fmt::Display,
{
    pub(crate) fn create(inner: F, config: C) -> Box<ServerServiceFactory<C> + Send> {
        Box::new(Self {
            inner,
            config,
            counter: Arc::new(AtomicUsize::new(0)),
        })
    }
}

pub trait ServerServiceFactory<C> {
    fn counter(&self) -> Arc<AtomicUsize>;

    fn clone_factory(&self) -> Box<ServerServiceFactory<C> + Send>;

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>>;
}

impl<F, T, C: Config> ServerServiceFactory<C> for ServerNewService<F, T, C>
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = TcpStream, Response = (), Config = C, InitError = io::Error>
        + 'static,
    T::Service: 'static,
    T::Future: 'static,
    T::Error: fmt::Display,
{
    fn counter(&self) -> Arc<AtomicUsize> {
        self.counter.clone()
    }

    fn clone_factory(&self) -> Box<ServerServiceFactory<C> + Send> {
        Box::new(Self {
            inner: self.inner.clone(),
            config: self.config.fork(),
            counter: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        let counter = self.counter.clone();
        Box::new(
            (self.inner)()
                .new_service(self.config.clone())
                .map_err(|_| ())
                .map(move |inner| {
                    let service: BoxedServerService =
                        Box::new(ServerService { inner, counter });
                    service
                }),
        )
    }
}

impl<C> ServerServiceFactory<C> for Box<ServerServiceFactory<C>> {
    fn counter(&self) -> Arc<AtomicUsize> {
        self.as_ref().counter()
    }

    fn clone_factory(&self) -> Box<ServerServiceFactory<C> + Send> {
        self.as_ref().clone_factory()
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        self.as_ref().create()
    }
}

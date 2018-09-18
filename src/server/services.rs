use std::net;

use futures::future::{err, ok};
use futures::{Future, Poll};
use tokio_reactor::Handle;
use tokio_tcp::TcpStream;

use service::{NewService, Service};

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

pub(crate) struct ServerNewService<F: ServerServiceFactory> {
    name: String,
    inner: F,
}

impl<F> ServerNewService<F>
where
    F: ServerServiceFactory,
{
    pub(crate) fn create(name: String, inner: F) -> Box<InternalServerServiceFactory> {
        Box::new(Self { name, inner })
    }
}

pub(crate) trait InternalServerServiceFactory: Send {
    fn name(&self) -> &str;

    fn clone_factory(&self) -> Box<InternalServerServiceFactory>;

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>>;
}

impl<F> InternalServerServiceFactory for ServerNewService<F>
where
    F: ServerServiceFactory,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn clone_factory(&self) -> Box<InternalServerServiceFactory> {
        Box::new(Self {
            name: self.name.clone(),
            inner: self.inner.clone(),
        })
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        Box::new(self.inner.create().new_service().map(move |inner| {
            let service: BoxedServerService = Box::new(ServerService::new(inner));
            service
        }))
    }
}

impl InternalServerServiceFactory for Box<InternalServerServiceFactory> {
    fn name(&self) -> &str {
        self.as_ref().name()
    }

    fn clone_factory(&self) -> Box<InternalServerServiceFactory> {
        self.as_ref().clone_factory()
    }

    fn create(&self) -> Box<Future<Item = BoxedServerService, Error = ()>> {
        self.as_ref().create()
    }
}

pub trait ServerServiceFactory: Send + Clone + 'static {
    type NewService: NewService<Request = TcpStream, Response = (), Error = (), InitError = ()>;

    fn create(&self) -> Self::NewService;
}

impl<F, T> ServerServiceFactory for F
where
    F: Fn() -> T + Send + Clone + 'static,
    T: NewService<Request = TcpStream, Response = (), Error = (), InitError = ()>,
{
    type NewService = T;

    fn create(&self) -> T {
        (self)()
    }
}

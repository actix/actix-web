use std::cell::Cell;
use std::net;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::{err, ok};
use futures::task::AtomicTask;
use futures::{Async, Future, Poll};
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

const MAX_CONNS: AtomicUsize = AtomicUsize::new(25600);

/// Sets the maximum per-worker number of concurrent connections.
///
/// All socket listeners will stop accepting connections when this limit is
/// reached for each worker.
///
/// By default max connections is set to a 25k per worker.
pub fn max_concurrent_connections(num: usize) {
    MAX_CONNS.store(num, Ordering::Relaxed);
}

pub(crate) fn num_connections() -> usize {
    MAX_CONNS_COUNTER.with(|counter| counter.total())
}

thread_local! {
    static MAX_CONNS_COUNTER: Counter = Counter::new(MAX_CONNS.load(Ordering::Relaxed));
}

pub(crate) struct ServerService<T> {
    service: T,
    counter: Counter,
}

impl<T> ServerService<T> {
    fn new(service: T) -> Self {
        MAX_CONNS_COUNTER.with(|counter| ServerService {
            service,
            counter: counter.clone(),
        })
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
        if self.counter.check() {
            self.service.poll_ready().map_err(|_| ())
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, req: ServerMessage) -> Self::Future {
        match req {
            ServerMessage::Connect(stream) => {
                let stream = TcpStream::from_std(stream, &Handle::default()).map_err(|e| {
                    error!("Can not convert to an async tcp stream: {}", e);
                });

                if let Ok(stream) = stream {
                    let guard = self.counter.get();

                    Box::new(
                        self.service
                            .call(stream)
                            .map_err(|_| ())
                            .map(move |_| drop(guard)),
                    )
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

#[derive(Clone)]
pub(crate) struct Counter(Rc<CounterInner>);

struct CounterInner {
    count: Cell<usize>,
    maxconn: usize,
    task: AtomicTask,
}

impl Counter {
    pub fn new(maxconn: usize) -> Self {
        Counter(Rc::new(CounterInner {
            maxconn,
            count: Cell::new(0),
            task: AtomicTask::new(),
        }))
    }

    pub fn get(&self) -> CounterGuard {
        CounterGuard::new(self.0.clone())
    }

    pub fn check(&self) -> bool {
        self.0.check()
    }

    pub fn total(&self) -> usize {
        self.0.count.get()
    }
}

pub(crate) struct CounterGuard(Rc<CounterInner>);

impl CounterGuard {
    fn new(inner: Rc<CounterInner>) -> Self {
        inner.inc();
        CounterGuard(inner)
    }
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

impl CounterInner {
    fn inc(&self) {
        let num = self.count.get() + 1;
        self.count.set(num);
        if num == self.maxconn {
            self.task.register();
        }
    }

    fn dec(&self) {
        let num = self.count.get();
        self.count.set(num - 1);
        if num == self.maxconn {
            self.task.notify();
        }
    }

    fn check(&self) -> bool {
        self.count.get() < self.maxconn
    }
}

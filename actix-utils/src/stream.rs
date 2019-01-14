use std::marker::PhantomData;
use std::rc::Rc;

use actix_rt::spawn;
use actix_service::{IntoNewService, IntoService, NewService, Service};
use futures::future::{ok, Future, FutureResult};
use futures::unsync::mpsc;
use futures::{Async, Poll, Stream};

type Request<T> = Result<<T as IntoStream>::Item, <T as IntoStream>::Error>;

pub trait IntoStream {
    type Item;
    type Error;
    type Stream: Stream<Item = Self::Item, Error = Self::Error>;

    fn into_stream(self) -> Self::Stream;
}

impl<T> IntoStream for T
where
    T: Stream,
{
    type Item = T::Item;
    type Error = T::Error;
    type Stream = T;

    fn into_stream(self) -> Self::Stream {
        self
    }
}

pub struct StreamNewService<S, T, E> {
    factory: Rc<T>,
    _t: PhantomData<(S, E)>,
}

impl<S, T, E> StreamNewService<S, T, E>
where
    S: IntoStream,
    T: NewService<Request<S>, Response = (), Error = E, InitError = E>,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service<Request<S>>>::Future: 'static,
{
    pub fn new<F: IntoNewService<T, Request<S>>>(factory: F) -> Self {
        Self {
            factory: Rc::new(factory.into_new_service()),
            _t: PhantomData,
        }
    }
}

impl<S, T, E> Clone for StreamNewService<S, T, E> {
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, T, E> NewService<S> for StreamNewService<S, T, E>
where
    S: IntoStream + 'static,
    T: NewService<Request<S>, Response = (), Error = E, InitError = E>,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service<Request<S>>>::Future: 'static,
{
    type Response = ();
    type Error = E;
    type InitError = E;
    type Service = StreamService<S, T, E>;
    type Future = FutureResult<Self::Service, E>;

    fn new_service(&self) -> Self::Future {
        ok(StreamService {
            factory: self.factory.clone(),
            _t: PhantomData,
        })
    }
}

pub struct StreamService<S, T, E> {
    factory: Rc<T>,
    _t: PhantomData<(S, E)>,
}

impl<S, T, E> Service<S> for StreamService<S, T, E>
where
    S: IntoStream + 'static,
    T: NewService<Request<S>, Response = (), Error = E, InitError = E>,
    T::Future: 'static,
    T::Service: 'static,
    <T::Service as Service<Request<S>>>::Future: 'static,
{
    type Response = ();
    type Error = E;
    type Future = Box<Future<Item = (), Error = E>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: S) -> Self::Future {
        Box::new(
            self.factory
                .new_service()
                .and_then(move |srv| StreamDispatcher::new(req, srv)),
        )
    }
}

pub struct StreamDispatcher<S, T>
where
    S: IntoStream + 'static,
    T: Service<Request<S>, Response = ()> + 'static,
    T::Future: 'static,
{
    stream: S,
    service: T,
    err_rx: mpsc::UnboundedReceiver<T::Error>,
    err_tx: mpsc::UnboundedSender<T::Error>,
}

impl<S, T> StreamDispatcher<S, T>
where
    S: Stream,
    T: Service<Request<S>, Response = ()>,
    T::Future: 'static,
{
    pub fn new<F1, F2>(stream: F1, service: F2) -> Self
    where
        F1: IntoStream<Stream = S, Item = S::Item, Error = S::Error>,
        F2: IntoService<T, Request<S>>,
    {
        let (err_tx, err_rx) = mpsc::unbounded();
        StreamDispatcher {
            err_rx,
            err_tx,
            stream: stream.into_stream(),
            service: service.into_service(),
        }
    }
}

impl<S, T> Future for StreamDispatcher<S, T>
where
    S: Stream,
    T: Service<Request<S>, Response = ()>,
    T::Future: 'static,
{
    type Item = ();
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Ok(Async::Ready(Some(e))) = self.err_rx.poll() {
            return Err(e);
        }

        loop {
            if let Async::Ready(_) = self.service.poll_ready()? {
                match self.stream.poll() {
                    Ok(Async::Ready(Some(item))) => spawn(StreamDispatcherService {
                        fut: self.service.call(Ok(item)),
                        stop: self.err_tx.clone(),
                    }),
                    Err(err) => spawn(StreamDispatcherService {
                        fut: self.service.call(Err(err)),
                        stop: self.err_tx.clone(),
                    }),
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Ok(Async::Ready(None)) => return Ok(Async::Ready(())),
                }
            }
        }
    }
}

struct StreamDispatcherService<F: Future> {
    fut: F,
    stop: mpsc::UnboundedSender<F::Error>,
}

impl<F: Future> Future for StreamDispatcherService<F> {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::Ready(_)) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                let _ = self.stop.unbounded_send(e);
                Ok(Async::Ready(()))
            }
        }
    }
}

/// `NewService` that implements, read one item from the stream.
pub struct TakeItem<T> {
    _t: PhantomData<T>,
}

impl<T> TakeItem<T> {
    /// Create new `TakeRequest` instance.
    pub fn new() -> Self {
        TakeItem { _t: PhantomData }
    }
}

impl<T> Default for TakeItem<T> {
    fn default() -> Self {
        TakeItem { _t: PhantomData }
    }
}

impl<T> Clone for TakeItem<T> {
    fn clone(&self) -> TakeItem<T> {
        TakeItem { _t: PhantomData }
    }
}

impl<T: Stream> NewService<T> for TakeItem<T> {
    type Response = (Option<T::Item>, T);
    type Error = T::Error;
    type InitError = ();
    type Service = TakeItemService<T>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(TakeItemService { _t: PhantomData })
    }
}

/// `NewService` that implements, read one request from framed object feature.
pub struct TakeItemService<T> {
    _t: PhantomData<T>,
}

impl<T> Clone for TakeItemService<T> {
    fn clone(&self) -> TakeItemService<T> {
        TakeItemService { _t: PhantomData }
    }
}

impl<T: Stream> Service<T> for TakeItemService<T> {
    type Response = (Option<T::Item>, T);
    type Error = T::Error;
    type Future = TakeItemServiceResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: T) -> Self::Future {
        TakeItemServiceResponse { stream: Some(req) }
    }
}

#[doc(hidden)]
pub struct TakeItemServiceResponse<T: Stream> {
    stream: Option<T>,
}

impl<T: Stream> Future for TakeItemServiceResponse<T> {
    type Item = (Option<T::Item>, T);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.stream.as_mut().expect("Use after finish").poll()? {
            Async::Ready(item) => Ok(Async::Ready((item, self.stream.take().unwrap()))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

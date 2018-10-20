use std::marker::PhantomData;

use futures::unsync::mpsc;
use futures::{future, Async, Future, Poll, Stream};
use tokio_current_thread::spawn;

use super::service::{IntoService, NewService, Service};

pub struct StreamDispatcher<S: Stream, T> {
    stream: S,
    service: T,
    item: Option<Result<S::Item, S::Error>>,
    stop_rx: mpsc::UnboundedReceiver<()>,
    stop_tx: mpsc::UnboundedSender<()>,
}

impl<S, T> StreamDispatcher<S, T>
where
    S: Stream,
    T: Service<Request = Result<S::Item, S::Error>, Response = (), Error = ()>,
    T::Future: 'static,
{
    pub fn new<F: IntoService<T>>(stream: S, service: F) -> Self {
        let (stop_tx, stop_rx) = mpsc::unbounded();
        StreamDispatcher {
            stream,
            item: None,
            service: service.into_service(),
            stop_rx,
            stop_tx,
        }
    }
}

impl<S, T> Future for StreamDispatcher<S, T>
where
    S: Stream,
    T: Service<Request = Result<S::Item, S::Error>, Response = (), Error = ()>,
    T::Future: 'static,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Ok(Async::Ready(Some(_))) = self.stop_rx.poll() {
            return Ok(Async::Ready(()));
        }

        let mut item = self.item.take();
        loop {
            if item.is_some() {
                match self.service.poll_ready()? {
                    Async::Ready(_) => spawn(StreamDispatcherService {
                        fut: self.service.call(item.take().unwrap()),
                        stop: self.stop_tx.clone(),
                    }),
                    Async::NotReady => {
                        self.item = item;
                        return Ok(Async::NotReady);
                    }
                }
            }
            match self.stream.poll() {
                Ok(Async::Ready(Some(el))) => item = Some(Ok(el)),
                Err(err) => item = Some(Err(err)),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Ok(Async::Ready(None)) => return Ok(Async::Ready(())),
            }
        }
    }
}

struct StreamDispatcherService<F: Future> {
    fut: F,
    stop: mpsc::UnboundedSender<()>,
}

impl<F: Future> Future for StreamDispatcherService<F> {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::Ready(_)) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(_) => {
                let _ = self.stop.unbounded_send(());
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

impl<T: Stream> NewService for TakeItem<T> {
    type Request = T;
    type Response = (Option<T::Item>, T);
    type Error = T::Error;
    type InitError = ();
    type Service = TakeItemService<T>;
    type Future = future::FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        future::ok(TakeItemService { _t: PhantomData })
    }
}

/// `NewService` that implements, read one request from framed object feature.
pub struct TakeItemService<T> {
    _t: PhantomData<T>,
}

impl<T: Stream> Service for TakeItemService<T> {
    type Request = T;
    type Response = (Option<T::Item>, T);
    type Error = T::Error;
    type Future = TakeItemServiceResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
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

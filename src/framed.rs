//! Framed dispatcher service and related utilities
use std::fmt;
use std::marker::PhantomData;

use actix;
use futures::future::{ok, Either, FutureResult};
use futures::unsync::mpsc;
use futures::{Async, AsyncSink, Future, Poll, Sink, Stream};
use tokio_codec::{Decoder, Encoder, Framed};
use tokio_io::{AsyncRead, AsyncWrite};

use service::{IntoNewService, IntoService, NewService, Service};

type Item<U> = <U as Encoder>::Item;
type StreamItem<U> = Result<<U as Decoder>::Item, <U as Decoder>::Error>;

pub struct FramedNewService<S, T, U> {
    factory: S,
    _t: PhantomData<(T, U)>,
}

impl<S, T, U> FramedNewService<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = StreamItem<U>, Response = Option<Item<U>>> + Clone,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: From<<U as Encoder>::Error> + 'static,
    <U as Encoder>::Item: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    pub fn new<F1: IntoNewService<S>>(factory: F1) -> Self {
        Self {
            factory: factory.into_new_service(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U> Clone for FramedNewService<S, T, U>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U> NewService for FramedNewService<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = StreamItem<U>, Response = Option<Item<U>>> + Clone,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: From<<U as Encoder>::Error> + 'static,
    <U as Encoder>::Item: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    type Request = Framed<T, U>;
    type Response = FramedDispatcher<S::Service, T, U>;
    type Error = S::InitError;
    type InitError = S::InitError;
    type Service = FramedService<S, T, U>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(FramedService {
            factory: self.factory.clone(),
            _t: PhantomData,
        })
    }
}

pub struct FramedService<S, T, U> {
    factory: S,
    _t: PhantomData<(T, U)>,
}

impl<S, T, U> Clone for FramedService<S, T, U>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U> Service for FramedService<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = StreamItem<U>, Response = Option<Item<U>>>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: From<<U as Encoder>::Error> + 'static,
    <U as Encoder>::Item: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    type Request = Framed<T, U>;
    type Response = FramedDispatcher<S::Service, T, U>;
    type Error = S::InitError;
    type Future = FramedServiceResponseFuture<S, T, U>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        FramedServiceResponseFuture {
            fut: self.factory.new_service(),
            framed: Some(req),
        }
    }
}

#[doc(hidden)]
pub struct FramedServiceResponseFuture<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = StreamItem<U>, Response = Option<Item<U>>>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: From<<U as Encoder>::Error> + 'static,
    <U as Encoder>::Item: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    fut: S::Future,
    framed: Option<Framed<T, U>>,
}

impl<S, T, U> Future for FramedServiceResponseFuture<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = StreamItem<U>, Response = Option<Item<U>>>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: From<<U as Encoder>::Error> + 'static,
    <U as Encoder>::Item: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    type Item = FramedDispatcher<S::Service, T, U>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(service) => Ok(Async::Ready(FramedDispatcher::new(
                self.framed.take().unwrap(),
                service,
            ))),
        }
    }
}

/// FramedDispatcher - is a future that reads frames from Framed object
/// and pass then to the service.
pub struct FramedDispatcher<S, T, U>
where
    S: Service,
    T: AsyncRead + AsyncWrite,
    U: Encoder + Decoder,
{
    service: S,
    framed: Framed<T, U>,
    item: Option<StreamItem<U>>,
    write_item: Option<Item<U>>,
    write_rx: mpsc::Receiver<Result<Item<U>, S::Error>>,
    write_tx: mpsc::Sender<Result<Item<U>, S::Error>>,
    flushed: bool,
}

impl<S, T, U> FramedDispatcher<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = StreamItem<U>, Response = Option<Item<U>>>,
    S::Future: 'static,
    S::Error: From<<U as Encoder>::Error> + 'static,
    <U as Encoder>::Item: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    pub fn new<F: IntoService<S>>(framed: Framed<T, U>, service: F) -> Self {
        let (write_tx, write_rx) = mpsc::channel(16);
        FramedDispatcher {
            framed,
            item: None,
            service: service.into_service(),
            write_rx,
            write_tx,
            write_item: None,
            flushed: true,
        }
    }
}

impl<S, T, U> Future for FramedDispatcher<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = StreamItem<U>, Response = Option<Item<U>>>,
    S::Future: 'static,
    S::Error: From<<U as Encoder>::Error> + 'static,
    <U as Encoder>::Item: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    type Item = ();
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Async::Ready(_) = self.service.poll_ready()? {
            let mut item = self.item.take();
            loop {
                if let Some(item) = item {
                    match self.service.poll_ready()? {
                        Async::Ready(_) => {
                            let sender = self.write_tx.clone();
                            actix::Arbiter::spawn(self.service.call(item).then(|item| {
                                let item = match item {
                                    Ok(item) => {
                                        if let Some(item) = item {
                                            Ok(item)
                                        } else {
                                            return Either::B(ok(()));
                                        }
                                    }
                                    Err(err) => Err(err),
                                };
                                Either::A(sender.send(item).map(|_| ()).map_err(|_| ()))
                            }));
                        }
                        Async::NotReady => {
                            self.item = Some(item);
                            break;
                        }
                    }
                }
                match self.framed.poll() {
                    Ok(Async::Ready(Some(el))) => item = Some(Ok(el)),
                    Err(err) => item = Some(Err(err)),
                    Ok(Async::NotReady) => break,
                    Ok(Async::Ready(None)) => return Ok(Async::Ready(())),
                }
            }
        }

        // write
        let mut item = self.write_item.take();
        loop {
            item = if let Some(msg) = item {
                self.flushed = false;
                match self.framed.start_send(msg) {
                    Ok(AsyncSink::Ready) => None,
                    Ok(AsyncSink::NotReady(item)) => Some(item),
                    Err(err) => {
                        trace!("Connection error: {:?}", err);
                        return Err(err.into());
                    }
                }
            } else {
                None
            };

            // flush sink
            if !self.flushed {
                match self.framed.poll_complete() {
                    Ok(Async::Ready(_)) => {
                        self.flushed = true;
                    }
                    Ok(Async::NotReady) => break,
                    Err(err) => {
                        trace!("Connection flush error: {:?}", err);
                        return Err(err.into());
                    }
                }
            }

            // check channel
            if self.flushed {
                if item.is_none() {
                    match self.write_rx.poll() {
                        Ok(Async::Ready(Some(msg))) => match msg {
                            Ok(msg) => item = Some(msg),
                            Err(err) => return Err(err),
                        },
                        Ok(Async::NotReady) => break,
                        Err(_) => panic!("Bug in gw code"),
                        Ok(Async::Ready(None)) => panic!("Bug in gw code"),
                    }
                } else {
                    continue;
                }
            } else {
                self.write_item = item;
                break;
            }
        }
        Ok(Async::NotReady)
    }
}

//! Framed dispatcher service and related utilities
use std::marker::PhantomData;
use std::mem;

use actix;
use futures::future::{ok, FutureResult};
use futures::unsync::mpsc;
use futures::{Async, AsyncSink, Future, Poll, Sink, Stream};
use tokio_codec::{Decoder, Encoder};
use tokio_io::{AsyncRead, AsyncWrite};

use codec::Framed;
use service::{IntoNewService, IntoService, NewService, Service};

type Request<U> = <U as Decoder>::Item;
type Response<U> = <U as Encoder>::Item;

pub struct FramedNewService<S, T, U> {
    factory: S,
    _t: PhantomData<(T, U)>,
}

impl<S, T, U> FramedNewService<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = Request<U>, Response = Response<U>> + Clone,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: 'static,
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
    S: NewService<Request = Request<U>, Response = Response<U>> + Clone,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: 'static,
{
    type Request = Framed<T, U>;
    type Response = FramedTransport<S::Service, T, U>;
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
    S: NewService<Request = Request<U>, Response = Response<U>>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: 'static,
{
    type Request = Framed<T, U>;
    type Response = FramedTransport<S::Service, T, U>;
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
    S: NewService<Request = Request<U>, Response = Response<U>>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: 'static,
{
    fut: S::Future,
    framed: Option<Framed<T, U>>,
}

impl<S, T, U> Future for FramedServiceResponseFuture<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = Request<U>, Response = Response<U>>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: 'static,
{
    type Item = FramedTransport<S::Service, T, U>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready(service) => Ok(Async::Ready(FramedTransport::new(
                self.framed.take().unwrap(),
                service,
            ))),
        }
    }
}

/// Framed transport errors
pub enum FramedTransportError<E, U: Encoder + Decoder> {
    Service(E),
    Encoder(<U as Encoder>::Error),
    Decoder(<U as Decoder>::Error),
}

impl<E, U: Encoder + Decoder> From<E> for FramedTransportError<E, U> {
    fn from(err: E) -> Self {
        FramedTransportError::Service(err)
    }
}

/// FramedTransport - is a future that reads frames from Framed object
/// and pass then to the service.
pub struct FramedTransport<S, T, U>
where
    S: Service,
    T: AsyncRead + AsyncWrite,
    U: Encoder + Decoder,
{
    service: S,
    state: TransportState<S, U>,
    framed: Framed<T, U>,
    request: Option<Request<U>>,
    response: Option<Response<U>>,
    write_rx: mpsc::Receiver<Result<Response<U>, S::Error>>,
    write_tx: mpsc::Sender<Result<Response<U>, S::Error>>,
    flushed: bool,
}

enum TransportState<S: Service, U: Encoder + Decoder> {
    Processing,
    Error(FramedTransportError<S::Error, U>),
    EncoderError(FramedTransportError<S::Error, U>),
    Stopping,
}

impl<S, T, U> FramedTransport<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = Request<U>, Response = Response<U>>,
    S::Future: 'static,
    S::Error: 'static,
    <U as Encoder>::Error: 'static,
{
    pub fn new<F: IntoService<S>>(framed: Framed<T, U>, service: F) -> Self {
        let (write_tx, write_rx) = mpsc::channel(16);
        FramedTransport {
            framed,
            write_rx,
            write_tx,
            service: service.into_service(),
            state: TransportState::Processing,
            request: None,
            response: None,
            flushed: true,
        }
    }

    /// Get reference to a service wrapped by `FramedTransport` instance.
    pub fn get_ref(&self) -> &S {
        &self.service
    }

    /// Get mutable reference to a service wrapped by `FramedTransport`
    /// instance.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.service
    }

    /// Get reference to a framed instance wrapped by `FramedTransport`
    /// instance.
    pub fn get_framed(&self) -> &Framed<T, U> {
        &self.framed
    }

    /// Get mutable reference to a framed instance wrapped by `FramedTransport`
    /// instance.
    pub fn get_framed_mut(&mut self) -> &mut Framed<T, U> {
        &mut self.framed
    }
}

impl<S, T, U> FramedTransport<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = Request<U>, Response = Response<U>>,
    S::Future: 'static,
    S::Error: 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: 'static,
{
    fn poll_service(&mut self) -> bool {
        match self.service.poll_ready() {
            Ok(Async::Ready(_)) => {
                if let Some(item) = self.request.take() {
                    let sender = self.write_tx.clone();
                    actix::Arbiter::spawn(
                        self.service
                            .call(item)
                            .then(|item| sender.send(item).map(|_| ()).map_err(|_| ())),
                    );
                }

                loop {
                    let item = match self.framed.poll() {
                        Ok(Async::Ready(Some(el))) => el,
                        Err(err) => {
                            self.state =
                                TransportState::Error(FramedTransportError::Decoder(err));
                            return true;
                        }
                        Ok(Async::NotReady) => return false,
                        Ok(Async::Ready(None)) => {
                            self.state = TransportState::Stopping;
                            return true;
                        }
                    };

                    match self.service.poll_ready() {
                        Ok(Async::Ready(_)) => {
                            let sender = self.write_tx.clone();
                            actix::Arbiter::spawn(
                                self.service
                                    .call(item)
                                    .then(|item| sender.send(item).map(|_| ()).map_err(|_| ())),
                            );
                        }
                        Ok(Async::NotReady) => {
                            self.request = Some(item);
                            return false;
                        }
                        Err(err) => {
                            self.state =
                                TransportState::Error(FramedTransportError::Service(err));
                            return true;
                        }
                    }
                }
            }
            Ok(Async::NotReady) => return false,
            Err(err) => {
                self.state = TransportState::Error(FramedTransportError::Service(err));
                return true;
            }
        }
    }

    /// write to sink
    fn poll_response(&mut self) -> bool {
        let mut item = self.response.take();
        loop {
            item = if let Some(msg) = item {
                self.flushed = false;
                match self.framed.start_send(msg) {
                    Ok(AsyncSink::Ready) => None,
                    Ok(AsyncSink::NotReady(item)) => Some(item),
                    Err(err) => {
                        self.state =
                            TransportState::EncoderError(FramedTransportError::Encoder(err));
                        return true;
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
                        self.state =
                            TransportState::EncoderError(FramedTransportError::Encoder(err));
                        return true;
                    }
                }
            }

            // check channel
            if self.flushed {
                if item.is_none() {
                    match self.write_rx.poll() {
                        Ok(Async::Ready(Some(msg))) => match msg {
                            Ok(msg) => item = Some(msg),
                            Err(err) => {
                                self.state =
                                    TransportState::Error(FramedTransportError::Service(err));
                                return true;
                            }
                        },
                        Ok(Async::NotReady) => break,
                        Err(_) => panic!("Bug in gw code"),
                        Ok(Async::Ready(None)) => panic!("Bug in gw code"),
                    }
                } else {
                    continue;
                }
            } else {
                self.response = item;
                break;
            }
        }

        false
    }
}

impl<S, T, U> Future for FramedTransport<S, T, U>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = Request<U>, Response = Response<U>>,
    S::Future: 'static,
    S::Error: 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: 'static,
{
    type Item = ();
    type Error = FramedTransportError<S::Error, U>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match mem::replace(&mut self.state, TransportState::Processing) {
            TransportState::Processing => {
                if self.poll_service() {
                    return self.poll();
                }
                if self.poll_response() {
                    return self.poll();
                }
                return Ok(Async::NotReady);
            }
            TransportState::Error(err) => {
                if self.poll_response() {
                    return Err(err);
                }
                if self.flushed {
                    return Err(err);
                }
                self.state = TransportState::Error(err);
                return Ok(Async::NotReady);
            }
            TransportState::EncoderError(err) => return Err(err),
            TransportState::Stopping => return Ok(Async::Ready(())),
        }
    }
}

pub struct IntoFramed<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    factory: F,
    _t: PhantomData<(T,)>,
}

impl<T, U, F> IntoFramed<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    pub fn new(factory: F) -> Self {
        IntoFramed {
            factory,
            _t: PhantomData,
        }
    }
}

impl<T, U, F> NewService for IntoFramed<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    type Request = T;
    type Response = Framed<T, U>;
    type Error = ();
    type InitError = ();
    type Service = IntoFramedService<T, U, F>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(IntoFramedService {
            factory: self.factory.clone(),
            _t: PhantomData,
        })
    }
}

pub struct IntoFramedService<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    factory: F,
    _t: PhantomData<(T,)>,
}

impl<T, U, F> Service for IntoFramedService<T, U, F>
where
    T: AsyncRead + AsyncWrite,
    F: Fn() -> U + Send + Clone + 'static,
    U: Encoder + Decoder,
{
    type Request = T;
    type Response = Framed<T, U>;
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        ok(Framed::new(req, (self.factory)()))
    }
}

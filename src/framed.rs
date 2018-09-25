//! Framed dispatcher service and related utilities
use std::fmt;
use std::marker::PhantomData;

use actix;
use futures::future::{ok, Either, FutureResult, Join};
use futures::unsync::mpsc;
use futures::{Async, AsyncSink, Future, Poll, Sink, Stream};
use tokio_codec::{Decoder, Encoder, Framed};
use tokio_io::{AsyncRead, AsyncWrite};

use service::{IntoNewService, IntoService, NewService, Service};

type Request<U> = <U as Decoder>::Item;
type Response<U> = <U as Encoder>::Item;

pub struct FramedNewService<S, T, U, E> {
    factory: S,
    error_handler: E,
    _t: PhantomData<(T, U)>,
}

impl<S, T, U> FramedNewService<S, T, U, DefaultErrorHandler<S, U, S::InitError>>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = Request<U>, Response = Option<Response<U>>> + Clone,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: fmt::Debug + 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    pub fn new<F1: IntoNewService<S>>(factory: F1) -> Self {
        Self {
            factory: factory.into_new_service(),
            error_handler: DefaultErrorHandler(PhantomData),
            _t: PhantomData,
        }
    }
}

impl<S, T, U, E> Clone for FramedNewService<S, T, U, E>
where
    S: Clone,
    E: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            error_handler: self.error_handler.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U, E> NewService for FramedNewService<S, T, U, E>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = Request<U>, Response = Option<Response<U>>> + Clone,
    E: NewService<Request = TransportError<S::Service, U>, InitError = S::InitError> + Clone,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: fmt::Debug + 'static,
    <U as Encoder>::Item: 'static,
    <U as Decoder>::Error: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    type Request = Framed<T, U>;
    type Response = FramedTransport<S::Service, T, U, E::Service>;
    type Error = S::InitError;
    type InitError = S::InitError;
    type Service = FramedService<S, T, U, E>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(FramedService {
            factory: self.factory.clone(),
            error_service: self.error_handler.clone(),
            _t: PhantomData,
        })
    }
}

pub struct FramedService<S, T, U, E> {
    factory: S,
    error_service: E,
    _t: PhantomData<(T, U)>,
}

impl<S, T, U, E> Clone for FramedService<S, T, U, E>
where
    S: Clone,
    E: Clone,
{
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
            error_service: self.error_service.clone(),
            _t: PhantomData,
        }
    }
}

impl<S, T, U, E> Service for FramedService<S, T, U, E>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = Request<U>, Response = Option<Response<U>>>,
    E: NewService<Request = TransportError<S::Service, U>, InitError = S::InitError>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: fmt::Debug + 'static,
    <U as Encoder>::Item: 'static,
    <U as Decoder>::Error: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    type Request = Framed<T, U>;
    type Response = FramedTransport<S::Service, T, U, E::Service>;
    type Error = S::InitError;
    type Future = FramedServiceResponseFuture<S, T, U, E>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        FramedServiceResponseFuture {
            fut: self
                .factory
                .new_service()
                .join(self.error_service.new_service()),
            framed: Some(req),
        }
    }
}

#[doc(hidden)]
pub struct FramedServiceResponseFuture<S, T, U, E>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = Request<U>, Response = Option<Response<U>>>,
    E: NewService<Request = TransportError<S::Service, U>, InitError = S::InitError>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: fmt::Debug + 'static,
    <U as Encoder>::Item: 'static,
    <U as Decoder>::Error: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    fut: Join<S::Future, E::Future>,
    framed: Option<Framed<T, U>>,
}

impl<S, T, U, E> Future for FramedServiceResponseFuture<S, T, U, E>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: NewService<Request = Request<U>, Response = Option<Response<U>>>,
    E: NewService<Request = TransportError<S::Service, U>, InitError = S::InitError>,
    <<S as NewService>::Service as Service>::Future: 'static,
    <<S as NewService>::Service as Service>::Error: fmt::Debug + 'static,
    <U as Encoder>::Item: 'static,
    <U as Decoder>::Error: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
{
    type Item = FramedTransport<S::Service, T, U, E::Service>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::NotReady => Ok(Async::NotReady),
            Async::Ready((service, error_service)) => {
                Ok(Async::Ready(FramedTransport::with_error_service(
                    self.framed.take().unwrap(),
                    service,
                    error_service,
                )))
            }
        }
    }
}

pub enum TransportError<S: Service, U: Encoder + Decoder> {
    Decoder(<U as Decoder>::Error),
    Encoder(<U as Encoder>::Error),
    Service(S::Error),
}

/// Default error handling service
pub struct DefaultErrorHandler<S, U, E>(PhantomData<(S, U, E)>);

impl<S, U, E> Service for DefaultErrorHandler<S, U, E>
where
    S: Service,
    U: Encoder + Decoder,
    S::Error: fmt::Debug,
    <U as Decoder>::Error: fmt::Debug,
    <U as Encoder>::Error: fmt::Debug,
{
    type Request = TransportError<S, U>;
    type Response = ();
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        match req {
            TransportError::Service(err) => debug!("Service error: {:?}", err),
            TransportError::Decoder(err) => trace!("Service decoder error: {:?}", err),
            TransportError::Encoder(err) => trace!("Service encoder error: {:?}", err),
        }
        ok(())
    }
}

impl<S, U, E> NewService for DefaultErrorHandler<S, U, E>
where
    S: Service,
    U: Encoder + Decoder,
    S::Error: fmt::Debug,
    <U as Decoder>::Error: fmt::Debug,
    <U as Encoder>::Error: fmt::Debug,
{
    type Request = TransportError<S, U>;
    type Response = ();
    type Error = ();
    type InitError = E;
    type Service = DefaultErrorHandler<S, U, ()>;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(DefaultErrorHandler(PhantomData))
    }
}

/// FramedTransport - is a future that reads frames from Framed object
/// and pass then to the service.
pub struct FramedTransport<S, T, U, E>
where
    S: Service,
    T: AsyncRead + AsyncWrite,
    U: Encoder + Decoder,
    E: Service,
{
    service: S,
    error_service: E,
    state: TransportState<E>,
    framed: Framed<T, U>,
    request: Option<Request<U>>,
    response: Option<Response<U>>,
    write_rx: mpsc::Receiver<Result<Response<U>, S::Error>>,
    write_tx: mpsc::Sender<Result<Response<U>, S::Error>>,
    flushed: bool,
}

enum TransportState<E: Service> {
    Processing,
    Error(E::Future),
    EncoderError(E::Future),
    SinkFlushing,
    Stopping,
}

impl<S, T, U> FramedTransport<S, T, U, DefaultErrorHandler<S, U, ()>>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = Request<U>, Response = Option<Response<U>>>,
    S::Future: 'static,
    S::Error: fmt::Debug + 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
    <U as Decoder>::Error: fmt::Debug + 'static,
{
    pub fn new<F: IntoService<S>>(framed: Framed<T, U>, service: F) -> Self {
        let (write_tx, write_rx) = mpsc::channel(16);
        FramedTransport {
            framed,
            write_rx,
            write_tx,
            service: service.into_service(),
            error_service: DefaultErrorHandler(PhantomData),
            state: TransportState::Processing,
            request: None,
            response: None,
            flushed: true,
        }
    }

    /// Set error handler service
    pub fn error_handler<E>(self, handler: E) -> FramedTransport<S, T, U, E>
    where
        E: Service<Request = TransportError<S, U>>,
    {
        FramedTransport {
            framed: self.framed,
            request: self.request,
            service: self.service,
            write_rx: self.write_rx,
            write_tx: self.write_tx,
            response: self.response,
            flushed: self.flushed,
            state: TransportState::Processing,
            error_service: handler,
        }
    }
}

impl<S, T, U, E> FramedTransport<S, T, U, E>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = Request<U>, Response = Option<Response<U>>>,
    E: Service<Request = TransportError<S, U>>,
    S::Future: 'static,
    S::Error: fmt::Debug + 'static,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
    <U as Decoder>::Error: fmt::Debug + 'static,
{
    pub fn with_error_service<F: IntoService<S>>(
        framed: Framed<T, U>, service: F, error_service: E,
    ) -> Self {
        let (write_tx, write_rx) = mpsc::channel(16);
        FramedTransport {
            framed,
            write_rx,
            write_tx,
            error_service,
            service: service.into_service(),
            state: TransportState::Processing,
            request: None,
            response: None,
            flushed: true,
        }
    }

    fn poll_service(&mut self) -> bool {
        match self.service.poll_ready() {
            Ok(Async::Ready(_)) => {
                let mut item = self.request.take();
                loop {
                    if let Some(item) = item {
                        match self.service.poll_ready() {
                            Ok(Async::Ready(_)) => {
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
                            Ok(Async::NotReady) => {
                                self.request = Some(item);
                                return false;
                            }
                            Err(err) => {
                                self.state = TransportState::Error(
                                    self.error_service.call(TransportError::Service(err)),
                                );
                                return true;
                            }
                        }
                    }
                    match self.framed.poll() {
                        Ok(Async::Ready(Some(el))) => item = Some(el),
                        Err(err) => {
                            self.state = TransportState::Error(
                                self.error_service.call(TransportError::Decoder(err)),
                            );
                            return true;
                        }
                        Ok(Async::NotReady) => return false,
                        Ok(Async::Ready(None)) => {
                            self.state = TransportState::Stopping;
                            return true;
                        }
                    }
                }
            }
            Ok(Async::NotReady) => return false,
            Err(err) => {
                self.state = TransportState::Error(
                    self.error_service.call(TransportError::Service(err)),
                );
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
                        trace!("Connection error: {:?}", err);
                        self.state = TransportState::EncoderError(
                            self.error_service.call(TransportError::Encoder(err)),
                        );
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
                        trace!("Connection flush error: {:?}", err);
                        self.state = TransportState::EncoderError(
                            self.error_service.call(TransportError::Encoder(err)),
                        );
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
                                self.state = TransportState::Error(
                                    self.error_service.call(TransportError::Service(err)),
                                );
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

impl<S, T, U, E> Future for FramedTransport<S, T, U, E>
where
    T: AsyncRead + AsyncWrite,
    U: Decoder + Encoder,
    S: Service<Request = Request<U>, Response = Option<Response<U>>>,
    S::Future: 'static,
    S::Error: fmt::Debug + 'static,
    E: Service<Request = TransportError<S, U>>,
    <U as Encoder>::Item: 'static,
    <U as Encoder>::Error: fmt::Debug + 'static,
    <U as Decoder>::Error: fmt::Debug + 'static,
{
    type Item = ();
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let state = match self.state {
            TransportState::Processing => {
                if self.poll_service() {
                    return self.poll();
                }
                if self.poll_response() {
                    return self.poll();
                }
                return Ok(Async::NotReady);
            }
            TransportState::Error(ref mut fut) => match fut.poll() {
                Err(_) | Ok(Async::Ready(_)) => TransportState::SinkFlushing,
                _ => return Ok(Async::NotReady),
            },
            TransportState::EncoderError(ref mut fut) => match fut.poll() {
                Err(_) | Ok(Async::Ready(_)) => return Ok(Async::Ready(())),
                _ => return Ok(Async::NotReady),
            },
            TransportState::SinkFlushing => {
                if self.poll_response() {
                    return self.poll();
                }
                if self.flushed {
                    return Ok(Async::Ready(()));
                }
                return Ok(Async::NotReady);
            }
            TransportState::Stopping => return Ok(Async::Ready(())),
        };

        self.state = state;
        self.poll()
    }
}

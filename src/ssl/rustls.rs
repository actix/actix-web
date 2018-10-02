use std::io;
use std::marker::PhantomData;
use std::sync::Arc;

use futures::{future::ok, future::FutureResult, Async, Future, Poll};
use rustls::{ServerConfig, ServerSession};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_rustls::{Accept, TlsAcceptor, TlsStream};

use super::MAX_CONN_COUNTER;
use counter::{Counter, CounterGuard};
use service::{NewService, Service};

/// Support `SSL` connections via rustls package
///
/// `rust-tls` feature enables `RustlsAcceptor` type
pub struct RustlsAcceptor<T> {
    config: Arc<ServerConfig>,
    io: PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite> RustlsAcceptor<T> {
    /// Create `RustlsAcceptor` new service
    pub fn new(config: ServerConfig) -> Self {
        RustlsAcceptor {
            config: Arc::new(config),
            io: PhantomData,
        }
    }
}

impl<T> Clone for RustlsAcceptor<T> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> NewService for RustlsAcceptor<T> {
    type Request = T;
    type Response = TlsStream<T, ServerSession>;
    type Error = io::Error;
    type Service = RustlsAcceptorService<T>;
    type InitError = ();
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ok(RustlsAcceptorService {
                acceptor: self.config.clone().into(),
                conns: conns.clone(),
                io: PhantomData,
            })
        })
    }
}

pub struct RustlsAcceptorService<T> {
    acceptor: TlsAcceptor,
    io: PhantomData<T>,
    conns: Counter,
}

impl<T: AsyncRead + AsyncWrite> Service for RustlsAcceptorService<T> {
    type Request = T;
    type Response = TlsStream<T, ServerSession>;
    type Error = io::Error;
    type Future = RustlsAcceptorServiceFut<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        if self.conns.available() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        RustlsAcceptorServiceFut {
            _guard: self.conns.get(),
            fut: self.acceptor.accept(req),
        }
    }
}

pub struct RustlsAcceptorServiceFut<T>
where
    T: AsyncRead + AsyncWrite,
{
    fut: Accept<T>,
    _guard: CounterGuard,
}

impl<T: AsyncRead + AsyncWrite> Future for RustlsAcceptorServiceFut<T> {
    type Item = TlsStream<T, ServerSession>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll()
    }
}

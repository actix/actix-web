use std::marker::PhantomData;

use actix_service::{NewService, Service};
use futures::{future::ok, future::FutureResult, Async, Future, Poll};
use openssl::ssl::{HandshakeError, SslAcceptor};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{AcceptAsync, SslAcceptorExt, SslStream};

use super::MAX_CONN_COUNTER;
use crate::counter::{Counter, CounterGuard};

/// Support `SSL` connections via openssl package
///
/// `ssl` feature enables `OpensslAcceptor` type
pub struct OpensslAcceptor<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
}

impl<T> OpensslAcceptor<T> {
    /// Create default `OpensslAcceptor`
    pub fn new(acceptor: SslAcceptor) -> Self {
        OpensslAcceptor {
            acceptor,
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> Clone for OpensslAcceptor<T> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> NewService<T> for OpensslAcceptor<T> {
    type Response = SslStream<T>;
    type Error = HandshakeError<T>;
    type Service = OpensslAcceptorService<T>;
    type InitError = ();
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        MAX_CONN_COUNTER.with(|conns| {
            ok(OpensslAcceptorService {
                acceptor: self.acceptor.clone(),
                conns: conns.clone(),
                io: PhantomData,
            })
        })
    }
}

pub struct OpensslAcceptorService<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
    conns: Counter,
}

impl<T: AsyncRead + AsyncWrite> Service<T> for OpensslAcceptorService<T> {
    type Response = SslStream<T>;
    type Error = HandshakeError<T>;
    type Future = OpensslAcceptorServiceFut<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        if self.conns.available() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, req: T) -> Self::Future {
        OpensslAcceptorServiceFut {
            _guard: self.conns.get(),
            fut: SslAcceptorExt::accept_async(&self.acceptor, req),
        }
    }
}

pub struct OpensslAcceptorServiceFut<T>
where
    T: AsyncRead + AsyncWrite,
{
    fut: AcceptAsync<T>,
    _guard: CounterGuard,
}

impl<T: AsyncRead + AsyncWrite> Future for OpensslAcceptorServiceFut<T> {
    type Item = SslStream<T>;
    type Error = HandshakeError<T>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll()
    }
}

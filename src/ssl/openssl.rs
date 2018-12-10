use std::marker::PhantomData;

use actix_service::{NewService, Service};
use futures::{future::ok, future::FutureResult, Async, Future, Poll};
use openssl::ssl::{Error, SslAcceptor, SslConnector};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{AcceptAsync, ConnectAsync, SslAcceptorExt, SslConnectorExt, SslStream};

use super::MAX_CONN_COUNTER;
use crate::counter::{Counter, CounterGuard};
use crate::resolver::RequestHost;

/// Openssl connector factory
pub struct OpensslConnector<R, T, E> {
    connector: SslConnector,
    _t: PhantomData<(R, T, E)>,
}

impl<R, T, E> OpensslConnector<R, T, E> {
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnector {
            connector,
            _t: PhantomData,
        }
    }
}

impl<R: RequestHost, T: AsyncRead + AsyncWrite> OpensslConnector<R, T, ()> {
    pub fn service(
        connector: SslConnector,
    ) -> impl Service<(R, T), Response = (R, SslStream<T>), Error = Error> {
        OpensslConnectorService {
            connector: connector,
            _t: PhantomData,
        }
    }
}

impl<R, T, E> Clone for OpensslConnector<R, T, E> {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
            _t: PhantomData,
        }
    }
}

impl<R: RequestHost, T: AsyncRead + AsyncWrite, E> NewService<(R, T)>
    for OpensslConnector<R, T, E>
{
    type Response = (R, SslStream<T>);
    type Error = Error;
    type Service = OpensslConnectorService<R, T>;
    type InitError = E;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(OpensslConnectorService {
            connector: self.connector.clone(),
            _t: PhantomData,
        })
    }
}

pub struct OpensslConnectorService<R, T> {
    connector: SslConnector,
    _t: PhantomData<(R, T)>,
}

impl<R: RequestHost, T: AsyncRead + AsyncWrite> Service<(R, T)>
    for OpensslConnectorService<R, T>
{
    type Response = (R, SslStream<T>);
    type Error = Error;
    type Future = ConnectAsyncExt<R, T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, stream): (R, T)) -> Self::Future {
        ConnectAsyncExt {
            fut: SslConnectorExt::connect_async(&self.connector, req.host(), stream),
            req: Some(req),
        }
    }
}

pub struct ConnectAsyncExt<R, T> {
    req: Option<R>,
    fut: ConnectAsync<T>,
}

impl<R, T> Future for ConnectAsyncExt<R, T>
where
    R: RequestHost,
    T: AsyncRead + AsyncWrite,
{
    type Item = (R, SslStream<T>);
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(stream) => Ok(Async::Ready((self.req.take().unwrap(), stream))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

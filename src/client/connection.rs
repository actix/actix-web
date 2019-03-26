use std::{fmt, time};

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::Bytes;
use futures::Future;
use h2::client::SendRequest;

use crate::body::MessageBody;
use crate::message::RequestHead;

use super::error::SendRequestError;
use super::pool::Acquired;
use super::response::ClientResponse;
use super::{h1proto, h2proto};

pub(crate) enum ConnectionType<Io> {
    H1(Io),
    H2(SendRequest<Bytes>),
}

pub trait Connection {
    type Future: Future<Item = ClientResponse, Error = SendRequestError>;

    /// Send request and body
    fn send_request<B: MessageBody + 'static>(
        self,
        head: RequestHead,
        body: B,
    ) -> Self::Future;
}

pub(crate) trait ConnectionLifetime: AsyncRead + AsyncWrite + 'static {
    /// Close connection
    fn close(&mut self);

    /// Release connection to the connection pool
    fn release(&mut self);
}

#[doc(hidden)]
/// HTTP client connection
pub struct IoConnection<T> {
    io: Option<ConnectionType<T>>,
    created: time::Instant,
    pool: Option<Acquired<T>>,
}

impl<T> fmt::Debug for IoConnection<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.io {
            Some(ConnectionType::H1(ref io)) => write!(f, "H1Connection({:?})", io),
            Some(ConnectionType::H2(_)) => write!(f, "H2Connection"),
            None => write!(f, "Connection(Empty)"),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> IoConnection<T> {
    pub(crate) fn new(
        io: ConnectionType<T>,
        created: time::Instant,
        pool: Option<Acquired<T>>,
    ) -> Self {
        IoConnection {
            pool,
            created,
            io: Some(io),
        }
    }

    pub(crate) fn into_inner(self) -> (ConnectionType<T>, time::Instant) {
        (self.io.unwrap(), self.created)
    }
}

impl<T> Connection for IoConnection<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    type Future = Box<Future<Item = ClientResponse, Error = SendRequestError>>;

    fn send_request<B: MessageBody + 'static>(
        mut self,
        head: RequestHead,
        body: B,
    ) -> Self::Future {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => Box::new(h1proto::send_request(
                io,
                head,
                body,
                self.created,
                self.pool,
            )),
            ConnectionType::H2(io) => Box::new(h2proto::send_request(
                io,
                head,
                body,
                self.created,
                self.pool,
            )),
        }
    }
}

#[allow(dead_code)]
pub(crate) enum EitherConnection<A, B> {
    A(IoConnection<A>),
    B(IoConnection<B>),
}

impl<A, B> Connection for EitherConnection<A, B>
where
    A: AsyncRead + AsyncWrite + 'static,
    B: AsyncRead + AsyncWrite + 'static,
{
    type Future = Box<Future<Item = ClientResponse, Error = SendRequestError>>;

    fn send_request<RB: MessageBody + 'static>(
        self,
        head: RequestHead,
        body: RB,
    ) -> Self::Future {
        match self {
            EitherConnection::A(con) => con.send_request(head, body),
            EitherConnection::B(con) => con.send_request(head, body),
        }
    }
}

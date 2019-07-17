use std::{fmt, io, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use bytes::{Buf, Bytes};
use futures::future::{err, Either, Future, FutureResult};
use futures::Poll;
use h2::client::SendRequest;

use crate::body::MessageBody;
use crate::h1::ClientCodec;
use crate::message::{RequestHead, ResponseHead};
use crate::payload::Payload;

use super::error::SendRequestError;
use super::pool::{Acquired, Protocol};
use super::{h1proto, h2proto};

pub(crate) enum ConnectionType<Io> {
    H1(Io),
    H2(SendRequest<Bytes>),
}

pub trait Connection {
    type Io: AsyncRead + AsyncWrite;
    type Future: Future<Item = (ResponseHead, Payload), Error = SendRequestError>;

    fn protocol(&self) -> Protocol;

    /// Send request and body
    fn send_request<B: MessageBody + 'static>(
        self,
        head: RequestHead,
        body: B,
    ) -> Self::Future;

    type TunnelFuture: Future<
        Item = (ResponseHead, Framed<Self::Io, ClientCodec>),
        Error = SendRequestError,
    >;

    /// Send request, returns Response and Framed
    fn open_tunnel(self, head: RequestHead) -> Self::TunnelFuture;
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

impl<T: AsyncRead + AsyncWrite> IoConnection<T> {
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
    type Io = T;
    type Future =
        Box<dyn Future<Item = (ResponseHead, Payload), Error = SendRequestError>>;

    fn protocol(&self) -> Protocol {
        match self.io {
            Some(ConnectionType::H1(_)) => Protocol::Http1,
            Some(ConnectionType::H2(_)) => Protocol::Http2,
            None => Protocol::Http1,
        }
    }

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

    type TunnelFuture = Either<
        Box<
            dyn Future<
                Item = (ResponseHead, Framed<Self::Io, ClientCodec>),
                Error = SendRequestError,
            >,
        >,
        FutureResult<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
    >;

    /// Send request, returns Response and Framed
    fn open_tunnel(mut self, head: RequestHead) -> Self::TunnelFuture {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => {
                Either::A(Box::new(h1proto::open_tunnel(io, head)))
            }
            ConnectionType::H2(io) => {
                if let Some(mut pool) = self.pool.take() {
                    pool.release(IoConnection::new(
                        ConnectionType::H2(io),
                        self.created,
                        None,
                    ));
                }
                Either::B(err(SendRequestError::TunnelNotSupported))
            }
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
    type Io = EitherIo<A, B>;
    type Future =
        Box<dyn Future<Item = (ResponseHead, Payload), Error = SendRequestError>>;

    fn protocol(&self) -> Protocol {
        match self {
            EitherConnection::A(con) => con.protocol(),
            EitherConnection::B(con) => con.protocol(),
        }
    }

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

    type TunnelFuture = Box<
        dyn Future<
            Item = (ResponseHead, Framed<Self::Io, ClientCodec>),
            Error = SendRequestError,
        >,
    >;

    /// Send request, returns Response and Framed
    fn open_tunnel(self, head: RequestHead) -> Self::TunnelFuture {
        match self {
            EitherConnection::A(con) => Box::new(
                con.open_tunnel(head)
                    .map(|(head, framed)| (head, framed.map_io(EitherIo::A))),
            ),
            EitherConnection::B(con) => Box::new(
                con.open_tunnel(head)
                    .map(|(head, framed)| (head, framed.map_io(EitherIo::B))),
            ),
        }
    }
}

pub enum EitherIo<A, B> {
    A(A),
    B(B),
}

impl<A, B> io::Read for EitherIo<A, B>
where
    A: io::Read,
    B: io::Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            EitherIo::A(ref mut val) => val.read(buf),
            EitherIo::B(ref mut val) => val.read(buf),
        }
    }
}

impl<A, B> AsyncRead for EitherIo<A, B>
where
    A: AsyncRead,
    B: AsyncRead,
{
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match self {
            EitherIo::A(ref val) => val.prepare_uninitialized_buffer(buf),
            EitherIo::B(ref val) => val.prepare_uninitialized_buffer(buf),
        }
    }
}

impl<A, B> io::Write for EitherIo<A, B>
where
    A: io::Write,
    B: io::Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            EitherIo::A(ref mut val) => val.write(buf),
            EitherIo::B(ref mut val) => val.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            EitherIo::A(ref mut val) => val.flush(),
            EitherIo::B(ref mut val) => val.flush(),
        }
    }
}

impl<A, B> AsyncWrite for EitherIo<A, B>
where
    A: AsyncWrite,
    B: AsyncWrite,
{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match self {
            EitherIo::A(ref mut val) => val.shutdown(),
            EitherIo::B(ref mut val) => val.shutdown(),
        }
    }

    fn write_buf<U: Buf>(&mut self, buf: &mut U) -> Poll<usize, io::Error>
    where
        Self: Sized,
    {
        match self {
            EitherIo::A(ref mut val) => val.write_buf(buf),
            EitherIo::B(ref mut val) => val.write_buf(buf),
        }
    }
}

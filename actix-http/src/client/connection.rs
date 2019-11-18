use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use bytes::{Buf, Bytes};
use futures::future::{err, Either, Future, FutureExt, LocalBoxFuture, Ready};
use h2::client::SendRequest;

use crate::body::MessageBody;
use crate::h1::ClientCodec;
use crate::message::{RequestHeadType, ResponseHead};
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
    type Future: Future<Output = Result<(ResponseHead, Payload), SendRequestError>>;

    fn protocol(&self) -> Protocol;

    /// Send request and body
    fn send_request<B: MessageBody + 'static, H: Into<RequestHeadType>>(
        self,
        head: H,
        body: B,
    ) -> Self::Future;

    type TunnelFuture: Future<
        Output = Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
    >;

    /// Send request, returns Response and Framed
    fn open_tunnel<H: Into<RequestHeadType>>(self, head: H) -> Self::TunnelFuture;
}

pub(crate) trait ConnectionLifetime:
    AsyncRead + AsyncWrite + Unpin + 'static
{
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
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Io = T;
    type Future =
        LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>;

    fn protocol(&self) -> Protocol {
        match self.io {
            Some(ConnectionType::H1(_)) => Protocol::Http1,
            Some(ConnectionType::H2(_)) => Protocol::Http2,
            None => Protocol::Http1,
        }
    }

    fn send_request<B: MessageBody + 'static, H: Into<RequestHeadType>>(
        mut self,
        head: H,
        body: B,
    ) -> Self::Future {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => {
                h1proto::send_request(io, head.into(), body, self.created, self.pool)
                    .boxed_local()
            }
            ConnectionType::H2(io) => {
                h2proto::send_request(io, head.into(), body, self.created, self.pool)
                    .boxed_local()
            }
        }
    }

    type TunnelFuture = Either<
        LocalBoxFuture<
            'static,
            Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
        >,
        Ready<Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>>,
    >;

    /// Send request, returns Response and Framed
    fn open_tunnel<H: Into<RequestHeadType>>(mut self, head: H) -> Self::TunnelFuture {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => {
                Either::Left(h1proto::open_tunnel(io, head.into()).boxed_local())
            }
            ConnectionType::H2(io) => {
                if let Some(mut pool) = self.pool.take() {
                    pool.release(IoConnection::new(
                        ConnectionType::H2(io),
                        self.created,
                        None,
                    ));
                }
                Either::Right(err(SendRequestError::TunnelNotSupported))
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
    A: AsyncRead + AsyncWrite + Unpin + 'static,
    B: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Io = EitherIo<A, B>;
    type Future =
        LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>;

    fn protocol(&self) -> Protocol {
        match self {
            EitherConnection::A(con) => con.protocol(),
            EitherConnection::B(con) => con.protocol(),
        }
    }

    fn send_request<RB: MessageBody + 'static, H: Into<RequestHeadType>>(
        self,
        head: H,
        body: RB,
    ) -> Self::Future {
        match self {
            EitherConnection::A(con) => con.send_request(head, body),
            EitherConnection::B(con) => con.send_request(head, body),
        }
    }

    type TunnelFuture = LocalBoxFuture<
        'static,
        Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
    >;

    /// Send request, returns Response and Framed
    fn open_tunnel<H: Into<RequestHeadType>>(self, head: H) -> Self::TunnelFuture {
        match self {
            EitherConnection::A(con) => con
                .open_tunnel(head)
                .map(|res| res.map(|(head, framed)| (head, framed.map_io(EitherIo::A))))
                .boxed_local(),
            EitherConnection::B(con) => con
                .open_tunnel(head)
                .map(|res| res.map(|(head, framed)| (head, framed.map_io(EitherIo::B))))
                .boxed_local(),
        }
    }
}

pub enum EitherIo<A, B> {
    A(A),
    B(B),
}

impl<A, B> AsyncRead for EitherIo<A, B>
where
    A: AsyncRead + Unpin,
    B: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            EitherIo::A(ref mut val) => Pin::new(val).poll_read(cx, buf),
            EitherIo::B(ref mut val) => Pin::new(val).poll_read(cx, buf),
        }
    }

    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match self {
            EitherIo::A(ref val) => val.prepare_uninitialized_buffer(buf),
            EitherIo::B(ref val) => val.prepare_uninitialized_buffer(buf),
        }
    }
}

impl<A, B> AsyncWrite for EitherIo<A, B>
where
    A: AsyncWrite + Unpin,
    B: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            EitherIo::A(ref mut val) => Pin::new(val).poll_write(cx, buf),
            EitherIo::B(ref mut val) => Pin::new(val).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            EitherIo::A(ref mut val) => Pin::new(val).poll_flush(cx),
            EitherIo::B(ref mut val) => Pin::new(val).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            EitherIo::A(ref mut val) => Pin::new(val).poll_shutdown(cx),
            EitherIo::B(ref mut val) => Pin::new(val).poll_shutdown(cx),
        }
    }

    fn poll_write_buf<U: Buf>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut U,
    ) -> Poll<Result<usize, io::Error>>
    where
        Self: Sized,
    {
        match self.get_mut() {
            EitherIo::A(ref mut val) => Pin::new(val).poll_write_buf(cx, buf),
            EitherIo::B(ref mut val) => Pin::new(val).poll_write_buf(cx, buf),
        }
    }
}

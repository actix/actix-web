use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io, time};

use actix_codec::{AsyncRead, AsyncWrite, Framed, ReadBuf};
use actix_rt::task::JoinHandle;
use bytes::Bytes;
use futures_core::future::LocalBoxFuture;
use h2::client::SendRequest;
use pin_project::pin_project;

use crate::body::MessageBody;
use crate::h1::ClientCodec;
use crate::message::{RequestHeadType, ResponseHead};
use crate::payload::Payload;

use super::error::SendRequestError;
use super::pool::{Acquired, Protocol};
use super::{h1proto, h2proto};

pub(crate) enum ConnectionType<Io> {
    H1(Io),
    H2(H2Connection),
}

// h2 connection has two parts: SendRequest and Connection.
// Connection is spawned as async task on runtime and H2Connection would hold a handle for
// this task. So it can wake up and quit the task when SendRequest is dropped.
pub(crate) struct H2Connection {
    handle: JoinHandle<()>,
    sender: SendRequest<Bytes>,
}

impl H2Connection {
    pub(crate) fn new<Io>(
        sender: SendRequest<Bytes>,
        connection: h2::client::Connection<Io>,
    ) -> Self
    where
        Io: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let handle = actix_rt::spawn(async move {
            let _ = connection.await;
        });

        Self { handle, sender }
    }
}

// wake up waker when drop
impl Drop for H2Connection {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

// only expose sender type to public.
impl Deref for H2Connection {
    type Target = SendRequest<Bytes>;

    fn deref(&self) -> &Self::Target {
        &self.sender
    }
}

impl DerefMut for H2Connection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.sender
    }
}

pub trait Connection {
    type Io: AsyncRead + AsyncWrite + Unpin;

    fn protocol(&self) -> Protocol;

    /// Send request and body
    fn send_request<B: MessageBody + 'static, H: Into<RequestHeadType>>(
        self,
        head: H,
        body: B,
    ) -> LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>;

    /// Send request, returns Response and Framed
    fn open_tunnel<H: Into<RequestHeadType> + 'static>(
        self,
        head: H,
    ) -> LocalBoxFuture<
        'static,
        Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
    >;
}

pub(crate) trait ConnectionLifetime: AsyncRead + AsyncWrite + 'static {
    /// Close connection
    fn close(self: Pin<&mut Self>);

    /// Release connection to the connection pool
    fn release(self: Pin<&mut Self>);
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.io {
            Some(ConnectionType::H1(ref io)) => write!(f, "H1Connection({:?})", io),
            Some(ConnectionType::H2(_)) => write!(f, "H2Connection"),
            None => write!(f, "Connection(Empty)"),
        }
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin> IoConnection<T> {
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
    ) -> LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>> {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => Box::pin(h1proto::send_request(
                io,
                head.into(),
                body,
                self.created,
                self.pool,
            )),
            ConnectionType::H2(io) => Box::pin(h2proto::send_request(
                io,
                head.into(),
                body,
                self.created,
                self.pool,
            )),
        }
    }

    /// Send request, returns Response and Framed
    fn open_tunnel<H: Into<RequestHeadType>>(
        mut self,
        head: H,
    ) -> LocalBoxFuture<
        'static,
        Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
    > {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => Box::pin(h1proto::open_tunnel(io, head.into())),
            ConnectionType::H2(io) => {
                if let Some(mut pool) = self.pool.take() {
                    pool.release(IoConnection::new(
                        ConnectionType::H2(io),
                        self.created,
                        None,
                    ));
                }
                Box::pin(async { Err(SendRequestError::TunnelNotSupported) })
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
    ) -> LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>> {
        match self {
            EitherConnection::A(con) => con.send_request(head, body),
            EitherConnection::B(con) => con.send_request(head, body),
        }
    }

    /// Send request, returns Response and Framed
    fn open_tunnel<H: Into<RequestHeadType> + 'static>(
        self,
        head: H,
    ) -> LocalBoxFuture<
        'static,
        Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
    > {
        match self {
            EitherConnection::A(con) => Box::pin(async {
                let (head, framed) = con.open_tunnel(head).await?;
                Ok((head, framed.into_map_io(EitherIo::A)))
            }),
            EitherConnection::B(con) => Box::pin(async {
                let (head, framed) = con.open_tunnel(head).await?;
                Ok((head, framed.into_map_io(EitherIo::B)))
            }),
        }
    }
}

#[pin_project(project = EitherIoProj)]
pub enum EitherIo<A, B> {
    A(#[pin] A),
    B(#[pin] B),
}

impl<A, B> AsyncRead for EitherIo<A, B>
where
    A: AsyncRead,
    B: AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.project() {
            EitherIoProj::A(val) => val.poll_read(cx, buf),
            EitherIoProj::B(val) => val.poll_read(cx, buf),
        }
    }
}

impl<A, B> AsyncWrite for EitherIo<A, B>
where
    A: AsyncWrite,
    B: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.project() {
            EitherIoProj::A(val) => val.poll_write(cx, buf),
            EitherIoProj::B(val) => val.poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.project() {
            EitherIoProj::A(val) => val.poll_flush(cx),
            EitherIoProj::B(val) => val.poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        match self.project() {
            EitherIoProj::A(val) => val.poll_shutdown(cx),
            EitherIoProj::B(val) => val.poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod test {
    use std::net;

    use actix_rt::net::TcpStream;

    use super::*;

    #[actix_rt::test]
    async fn test_h2_connection_drop() {
        let addr = "127.0.0.1:0".parse::<net::SocketAddr>().unwrap();
        let listener = net::TcpListener::bind(addr).unwrap();
        let local = listener.local_addr().unwrap();

        std::thread::spawn(move || while listener.accept().is_ok() {});

        let tcp = TcpStream::connect(local).await.unwrap();
        let (sender, connection) = h2::client::handshake(tcp).await.unwrap();
        let conn = H2Connection::new(sender.clone(), connection);

        assert!(sender.clone().ready().await.is_ok());
        assert!(h2::client::SendRequest::clone(&*conn).ready().await.is_ok());

        drop(conn);

        match sender.ready().await {
            Ok(_) => panic!("connection should be gone and can not be ready"),
            Err(e) => assert!(e.is_io()),
        };
    }
}

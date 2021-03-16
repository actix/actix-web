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
use super::pool::Acquired;
use super::{h1proto, h2proto};

pub(crate) enum ConnectionType<Io> {
    H1(Io),
    H2(H2Connection),
}

/// `H2Connection` has two parts: `SendRequest` and `Connection`.
///
/// `Connection` is spawned as an async task on runtime and `H2Connection` holds a handle for
/// this task. Therefore, it can wake up and quit the task when SendRequest is dropped.
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

// cancel spawned connection task on drop.
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

    /// Send request and body
    fn send_request<B, H>(
        self,
        head: H,
        body: B,
    ) -> LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>
    where
        B: MessageBody + 'static,
        H: Into<RequestHeadType> + 'static;

    /// Send request, returns Response and Framed
    fn open_tunnel<H: Into<RequestHeadType> + 'static>(
        self,
        head: H,
    ) -> LocalBoxFuture<
        'static,
        Result<(ResponseHead, Framed<Self::Io, ClientCodec>), SendRequestError>,
    >;
}

#[doc(hidden)]
/// HTTP client connection
pub struct IoConnection<T>
where
    T: AsyncWrite + Unpin + 'static,
{
    io: Option<ConnectionType<T>>,
    created: time::Instant,
    pool: Acquired<T>,
}

impl<T> fmt::Debug for IoConnection<T>
where
    T: AsyncWrite + Unpin + fmt::Debug + 'static,
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
        pool: Acquired<T>,
    ) -> Self {
        IoConnection {
            pool,
            created,
            io: Some(io),
        }
    }

    #[cfg(test)]
    pub(crate) fn into_parts(self) -> (ConnectionType<T>, time::Instant, Acquired<T>) {
        (self.io.unwrap(), self.created, self.pool)
    }

    async fn send_request<B: MessageBody + 'static, H: Into<RequestHeadType>>(
        mut self,
        head: H,
        body: B,
    ) -> Result<(ResponseHead, Payload), SendRequestError> {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => {
                h1proto::send_request(io, head.into(), body, self.created, self.pool)
                    .await
            }
            ConnectionType::H2(io) => {
                h2proto::send_request(io, head.into(), body, self.created, self.pool)
                    .await
            }
        }
    }

    /// Send request, returns Response and Framed
    async fn open_tunnel<H: Into<RequestHeadType>>(
        mut self,
        head: H,
    ) -> Result<(ResponseHead, Framed<T, ClientCodec>), SendRequestError> {
        match self.io.take().unwrap() {
            ConnectionType::H1(io) => h1proto::open_tunnel(io, head.into()).await,
            ConnectionType::H2(io) => {
                self.pool.release(ConnectionType::H2(io), self.created);
                Err(SendRequestError::TunnelNotSupported)
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) enum EitherIoConnection<A, B>
where
    A: AsyncRead + AsyncWrite + Unpin + 'static,
    B: AsyncRead + AsyncWrite + Unpin + 'static,
{
    A(IoConnection<A>),
    B(IoConnection<B>),
}

impl<A, B> Connection for EitherIoConnection<A, B>
where
    A: AsyncRead + AsyncWrite + Unpin + 'static,
    B: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Io = EitherIo<A, B>;

    fn send_request<RB, H>(
        self,
        head: H,
        body: RB,
    ) -> LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>
    where
        RB: MessageBody + 'static,
        H: Into<RequestHeadType> + 'static,
    {
        match self {
            EitherIoConnection::A(con) => Box::pin(con.send_request(head, body)),
            EitherIoConnection::B(con) => Box::pin(con.send_request(head, body)),
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
            EitherIoConnection::A(con) => Box::pin(async {
                let (head, framed) = con.open_tunnel(head).await?;
                Ok((head, framed.into_map_io(EitherIo::A)))
            }),
            EitherIoConnection::B(con) => Box::pin(async {
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

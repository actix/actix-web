use std::{
    io,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
    time,
};

use actix_codec::{AsyncRead, AsyncWrite, Framed, ReadBuf};
use actix_http::{body::MessageBody, h1::ClientCodec, Payload, RequestHeadType, ResponseHead};
use actix_rt::task::JoinHandle;
use bytes::Bytes;
use futures_core::future::LocalBoxFuture;
use h2::client::SendRequest;

use super::{error::SendRequestError, h1proto, h2proto, pool::Acquired};
use crate::BoxError;

/// Trait alias for types impl [tokio::io::AsyncRead] and [tokio::io::AsyncWrite].
pub trait ConnectionIo: AsyncRead + AsyncWrite + Unpin + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + 'static> ConnectionIo for T {}

/// HTTP client connection
pub struct H1Connection<Io: ConnectionIo> {
    io: Option<Io>,
    created: time::Instant,
    acquired: Acquired<Io>,
}

impl<Io: ConnectionIo> H1Connection<Io> {
    /// close or release the connection to pool based on flag input
    pub(super) fn on_release(&mut self, keep_alive: bool) {
        if keep_alive {
            self.release();
        } else {
            self.close();
        }
    }

    /// Close connection
    fn close(&mut self) {
        let io = self.io.take().unwrap();
        self.acquired.close(ConnectionInnerType::H1(io));
    }

    /// Release this connection to the connection pool
    fn release(&mut self) {
        let io = self.io.take().unwrap();
        self.acquired
            .release(ConnectionInnerType::H1(io), self.created);
    }

    fn io_pin_mut(self: Pin<&mut Self>) -> Pin<&mut Io> {
        Pin::new(self.get_mut().io.as_mut().unwrap())
    }
}

impl<Io: ConnectionIo> AsyncRead for H1Connection<Io> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.io_pin_mut().poll_read(cx, buf)
    }
}

impl<Io: ConnectionIo> AsyncWrite for H1Connection<Io> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.io_pin_mut().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.io_pin_mut().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.io_pin_mut().poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.io_pin_mut().poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.as_ref().unwrap().is_write_vectored()
    }
}

/// HTTP2 client connection
pub struct H2Connection<Io: ConnectionIo> {
    io: Option<H2ConnectionInner>,
    created: time::Instant,
    acquired: Acquired<Io>,
}

impl<Io: ConnectionIo> Deref for H2Connection<Io> {
    type Target = SendRequest<Bytes>;

    fn deref(&self) -> &Self::Target {
        &self.io.as_ref().unwrap().sender
    }
}

impl<Io: ConnectionIo> DerefMut for H2Connection<Io> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.io.as_mut().unwrap().sender
    }
}

impl<Io: ConnectionIo> H2Connection<Io> {
    /// close or release the connection to pool based on flag input
    pub(super) fn on_release(&mut self, close: bool) {
        if close {
            self.close();
        } else {
            self.release();
        }
    }

    /// Close connection
    fn close(&mut self) {
        let io = self.io.take().unwrap();
        self.acquired.close(ConnectionInnerType::H2(io));
    }

    /// Release this connection to the connection pool
    fn release(&mut self) {
        let io = self.io.take().unwrap();
        self.acquired
            .release(ConnectionInnerType::H2(io), self.created);
    }
}

/// `H2ConnectionInner` has two parts: `SendRequest` and `Connection`.
///
/// `Connection` is spawned as an async task on runtime and `H2ConnectionInner` holds a handle
/// for this task. Therefore, it can wake up and quit the task when SendRequest is dropped.
pub(super) struct H2ConnectionInner {
    handle: JoinHandle<()>,
    sender: SendRequest<Bytes>,
}

impl H2ConnectionInner {
    pub(super) fn new<Io: ConnectionIo>(
        sender: SendRequest<Bytes>,
        connection: h2::client::Connection<Io>,
    ) -> Self {
        let handle = actix_rt::spawn(async move {
            let _ = connection.await;
        });

        Self { handle, sender }
    }
}

/// Cancel spawned connection task on drop.
impl Drop for H2ConnectionInner {
    fn drop(&mut self) {
        // TODO: this can end up sending extraneous requests; see if there is a better way to handle
        if self
            .sender
            .send_request(http::Request::new(()), true)
            .is_err()
        {
            self.handle.abort();
        }
    }
}

/// Unified connection type cover HTTP/1 Plain/TLS and HTTP/2 protocols.
#[allow(dead_code)]
pub enum Connection<A, B = Box<dyn ConnectionIo>>
where
    A: ConnectionIo,
    B: ConnectionIo,
{
    Tcp(ConnectionType<A>),
    Tls(ConnectionType<B>),
}

/// Unified connection type cover Http1/2 protocols
pub enum ConnectionType<Io: ConnectionIo> {
    H1(H1Connection<Io>),
    H2(H2Connection<Io>),
}

/// Helper type for storing connection types in pool.
pub(super) enum ConnectionInnerType<Io> {
    H1(Io),
    H2(H2ConnectionInner),
}

impl<Io: ConnectionIo> ConnectionType<Io> {
    pub(super) fn from_pool(
        inner: ConnectionInnerType<Io>,
        created: time::Instant,
        acquired: Acquired<Io>,
    ) -> Self {
        match inner {
            ConnectionInnerType::H1(io) => Self::from_h1(io, created, acquired),
            ConnectionInnerType::H2(io) => Self::from_h2(io, created, acquired),
        }
    }

    pub(super) fn from_h1(io: Io, created: time::Instant, acquired: Acquired<Io>) -> Self {
        Self::H1(H1Connection {
            io: Some(io),
            created,
            acquired,
        })
    }

    pub(super) fn from_h2(
        io: H2ConnectionInner,
        created: time::Instant,
        acquired: Acquired<Io>,
    ) -> Self {
        Self::H2(H2Connection {
            io: Some(io),
            created,
            acquired,
        })
    }
}

impl<A, B> Connection<A, B>
where
    A: ConnectionIo,
    B: ConnectionIo,
{
    /// Send a request through connection.
    pub fn send_request<RB, H>(
        self,
        head: H,
        body: RB,
    ) -> LocalBoxFuture<'static, Result<(ResponseHead, Payload), SendRequestError>>
    where
        H: Into<RequestHeadType> + 'static,
        RB: MessageBody + 'static,
        RB::Error: Into<BoxError>,
    {
        Box::pin(async move {
            match self {
                Connection::Tcp(ConnectionType::H1(conn)) => {
                    h1proto::send_request(conn, head.into(), body).await
                }
                Connection::Tls(ConnectionType::H1(conn)) => {
                    h1proto::send_request(conn, head.into(), body).await
                }
                Connection::Tls(ConnectionType::H2(conn)) => {
                    h2proto::send_request(conn, head.into(), body).await
                }
                _ => {
                    unreachable!("Plain TCP connection can be used only with HTTP/1.1 protocol")
                }
            }
        })
    }

    /// Send request, returns Response and Framed tunnel.
    pub fn open_tunnel<H: Into<RequestHeadType> + 'static>(
        self,
        head: H,
    ) -> LocalBoxFuture<
        'static,
        Result<(ResponseHead, Framed<Connection<A, B>, ClientCodec>), SendRequestError>,
    > {
        Box::pin(async move {
            match self {
                Connection::Tcp(ConnectionType::H1(ref _conn)) => {
                    let (head, framed) = h1proto::open_tunnel(self, head.into()).await?;
                    Ok((head, framed))
                }
                Connection::Tls(ConnectionType::H1(ref _conn)) => {
                    let (head, framed) = h1proto::open_tunnel(self, head.into()).await?;
                    Ok((head, framed))
                }
                Connection::Tls(ConnectionType::H2(mut conn)) => {
                    conn.release();
                    Err(SendRequestError::TunnelNotSupported)
                }
                Connection::Tcp(ConnectionType::H2(_)) => {
                    unreachable!("Plain Tcp connection can be used only in Http1 protocol")
                }
            }
        })
    }
}

impl<A, B> AsyncRead for Connection<A, B>
where
    A: ConnectionIo,
    B: ConnectionIo,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Connection::Tcp(ConnectionType::H1(conn)) => Pin::new(conn).poll_read(cx, buf),
            Connection::Tls(ConnectionType::H1(conn)) => Pin::new(conn).poll_read(cx, buf),
            _ => unreachable!("H2Connection can not impl AsyncRead trait"),
        }
    }
}

const H2_UNREACHABLE_WRITE: &str = "H2Connection can not impl AsyncWrite trait";

impl<A, B> AsyncWrite for Connection<A, B>
where
    A: ConnectionIo,
    B: ConnectionIo,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Connection::Tcp(ConnectionType::H1(conn)) => Pin::new(conn).poll_write(cx, buf),
            Connection::Tls(ConnectionType::H1(conn)) => Pin::new(conn).poll_write(cx, buf),
            _ => unreachable!("{}", H2_UNREACHABLE_WRITE),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Connection::Tcp(ConnectionType::H1(conn)) => Pin::new(conn).poll_flush(cx),
            Connection::Tls(ConnectionType::H1(conn)) => Pin::new(conn).poll_flush(cx),
            _ => unreachable!("{}", H2_UNREACHABLE_WRITE),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Connection::Tcp(ConnectionType::H1(conn)) => Pin::new(conn).poll_shutdown(cx),
            Connection::Tls(ConnectionType::H1(conn)) => Pin::new(conn).poll_shutdown(cx),
            _ => unreachable!("{}", H2_UNREACHABLE_WRITE),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Connection::Tcp(ConnectionType::H1(conn)) => {
                Pin::new(conn).poll_write_vectored(cx, bufs)
            }
            Connection::Tls(ConnectionType::H1(conn)) => {
                Pin::new(conn).poll_write_vectored(cx, bufs)
            }
            _ => unreachable!("{}", H2_UNREACHABLE_WRITE),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match *self {
            Connection::Tcp(ConnectionType::H1(ref conn)) => conn.is_write_vectored(),
            Connection::Tls(ConnectionType::H1(ref conn)) => conn.is_write_vectored(),
            _ => unreachable!("{}", H2_UNREACHABLE_WRITE),
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        future::Future,
        net,
        time::{Duration, Instant},
    };

    use actix_rt::{
        net::TcpStream,
        time::{interval, Interval},
    };

    use super::*;

    #[actix_rt::test]
    async fn test_h2_connection_drop() {
        env_logger::try_init().ok();

        let addr = "127.0.0.1:0".parse::<net::SocketAddr>().unwrap();
        let listener = net::TcpListener::bind(addr).unwrap();
        let local = listener.local_addr().unwrap();

        std::thread::spawn(move || while listener.accept().is_ok() {});

        let tcp = TcpStream::connect(local).await.unwrap();
        let (sender, connection) = h2::client::handshake(tcp).await.unwrap();
        let conn = H2ConnectionInner::new(sender.clone(), connection);

        assert!(sender.clone().ready().await.is_ok());
        assert!(h2::client::SendRequest::clone(&conn.sender)
            .ready()
            .await
            .is_ok());

        drop(conn);

        struct DropCheck {
            sender: h2::client::SendRequest<Bytes>,
            interval: Interval,
            start_from: Instant,
        }

        impl Future for DropCheck {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let this = self.get_mut();
                match futures_core::ready!(this.sender.poll_ready(cx)) {
                    Ok(()) => {
                        if this.start_from.elapsed() > Duration::from_secs(10) {
                            panic!("connection should be gone and can not be ready");
                        } else {
                            match this.interval.poll_tick(cx) {
                                Poll::Ready(_) => {
                                    // prevents spurious test hang
                                    this.interval.reset();

                                    Poll::Pending
                                }
                                Poll::Pending => Poll::Pending,
                            }
                        }
                    }
                    Err(_) => Poll::Ready(()),
                }
            }
        }

        DropCheck {
            sender,
            interval: interval(Duration::from_millis(100)),
            start_from: Instant::now(),
        }
        .await;
    }
}

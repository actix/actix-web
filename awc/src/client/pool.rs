//! Client connection pooling keyed on the authority part of the connection URI.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::Future,
    io,
    ops::Deref,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use actix_http::Protocol;
use actix_rt::time::{sleep, Sleep};
use actix_service::Service;
use futures_core::future::LocalBoxFuture;
use futures_util::FutureExt as _;
use http::uri::Authority;
use pin_project_lite::pin_project;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{
    config::ConnectorConfig,
    connection::{ConnectionInnerType, ConnectionIo, ConnectionType, H2ConnectionInner},
    error::ConnectError,
    h2proto::handshake,
    Connect,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    authority: Authority,
}

impl From<Authority> for Key {
    fn from(authority: Authority) -> Key {
        Key { authority }
    }
}

/// Connections pool to reuse I/O per [`Authority`].
#[doc(hidden)]
pub struct ConnectionPool<S, Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    connector: S,
    inner: ConnectionPoolInner<Io>,
}

/// Wrapper type for check the ref count of Rc.
pub struct ConnectionPoolInner<Io>(Rc<ConnectionPoolInnerPriv<Io>>)
where
    Io: AsyncWrite + Unpin + 'static;

impl<Io> ConnectionPoolInner<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    fn new(config: ConnectorConfig) -> Self {
        let permits = Arc::new(Semaphore::new(config.limit));
        let available = RefCell::new(HashMap::new());

        Self(Rc::new(ConnectionPoolInnerPriv {
            config,
            available,
            permits,
        }))
    }

    /// Spawns a graceful shutdown task for the underlying I/O with a timeout.
    fn close(&self, conn: ConnectionInnerType<Io>) {
        if let Some(timeout) = self.config.disconnect_timeout {
            if let ConnectionInnerType::H1(io) = conn {
                if tokio::runtime::Handle::try_current().is_ok() {
                    actix_rt::spawn(CloseConnection::new(io, timeout));
                }
            }
        }
    }
}

impl<Io> Clone for ConnectionPoolInner<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<Io> Deref for ConnectionPoolInner<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    type Target = ConnectionPoolInnerPriv<Io>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<Io> Drop for ConnectionPoolInner<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    fn drop(&mut self) {
        // When strong count is one it means the pool is dropped
        // remove and drop all Io types.
        if Rc::strong_count(&self.0) == 1 {
            self.permits.close();
            std::mem::take(&mut *self.available.borrow_mut())
                .into_iter()
                .for_each(|(_, conns)| {
                    conns.into_iter().for_each(|pooled| self.close(pooled.conn))
                });
        }
    }
}

pub struct ConnectionPoolInnerPriv<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    config: ConnectorConfig,
    available: RefCell<HashMap<Key, VecDeque<PooledConnection<Io>>>>,
    permits: Arc<Semaphore>,
}

impl<S, Io> ConnectionPool<S, Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    /// Construct a new connection pool.
    ///
    /// [`super::config::ConnectorConfig`]'s `limit` is used as the max permits allowed for
    /// in-flight connections.
    ///
    /// The pool can only have equal to `limit` amount of requests spawning/using Io type
    /// concurrently.
    ///
    /// Any requests beyond limit would be wait in fifo order and get notified in async manner
    /// by [`tokio::sync::Semaphore`]
    pub(crate) fn new(connector: S, config: ConnectorConfig) -> Self {
        let inner = ConnectionPoolInner::new(config);

        Self { connector, inner }
    }
}

impl<S, Io> Service<Connect> for ConnectionPool<S, Io>
where
    S: Service<Connect, Response = (Io, Protocol), Error = ConnectError> + Clone + 'static,
    Io: ConnectionIo,
{
    type Response = ConnectionType<Io>;
    type Error = ConnectError;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::forward_ready!(connector);

    fn call(&self, req: Connect) -> Self::Future {
        let connector = self.connector.clone();
        let inner = self.inner.clone();

        Box::pin(async move {
            let key = if let Some(authority) = req.uri.authority() {
                authority.clone().into()
            } else {
                return Err(ConnectError::Unresolved);
            };

            // acquire an owned permit and carry it with connection
            let permit = Arc::clone(&inner.permits)
                .acquire_owned()
                .await
                .map_err(|_| {
                    ConnectError::Io(io::Error::new(
                        io::ErrorKind::Other,
                        "failed to acquire semaphore on client connection pool",
                    ))
                })?;

            let conn = {
                let mut conn = None;

                // check if there is idle connection for given key.
                let mut map = inner.available.borrow_mut();

                if let Some(conns) = map.get_mut(&key) {
                    let now = Instant::now();

                    while let Some(mut c) = conns.pop_front() {
                        let config = &inner.config;
                        let idle_dur = now - c.used;
                        let age = now - c.created;
                        let conn_ineligible =
                            idle_dur > config.conn_keep_alive || age > config.conn_lifetime;

                        if conn_ineligible {
                            // drop connections that are too old
                            inner.close(c.conn);
                        } else {
                            // check if the connection is still usable
                            if let ConnectionInnerType::H1(ref mut io) = c.conn {
                                let check = ConnectionCheckFuture { io };
                                match check.now_or_never().expect(
                                    "ConnectionCheckFuture must never yield with Poll::Pending.",
                                ) {
                                    ConnectionState::Tainted => {
                                        inner.close(c.conn);
                                        continue;
                                    }
                                    ConnectionState::Skip => continue,
                                    ConnectionState::Live => conn = Some(c),
                                }
                            } else {
                                conn = Some(c);
                            }

                            break;
                        }
                    }
                };

                conn
            };

            // construct acquired. It's used to put Io type back to pool/ close the Io type.
            // permit is carried with the whole lifecycle of Acquired.
            let acquired = Acquired { key, inner, permit };

            // match the connection and spawn new one if did not get anything.
            match conn {
                Some(conn) => Ok(ConnectionType::from_pool(conn.conn, conn.created, acquired)),
                None => {
                    let (io, proto) = connector.call(req).await?;

                    // NOTE: remove when http3 is added in support.
                    assert!(proto != Protocol::Http3);

                    if proto == Protocol::Http1 {
                        Ok(ConnectionType::from_h1(io, Instant::now(), acquired))
                    } else {
                        let config = &acquired.inner.config;
                        let (sender, connection) = handshake(io, config).await?;
                        let inner = H2ConnectionInner::new(sender, connection);
                        Ok(ConnectionType::from_h2(inner, Instant::now(), acquired))
                    }
                }
            }
        })
    }
}

/// Type for check the connection and determine if it's usable.
struct ConnectionCheckFuture<'a, Io> {
    io: &'a mut Io,
}

enum ConnectionState {
    /// IO is pending and a new request would wake it.
    Live,

    /// IO unexpectedly has unread data and should be dropped.
    Tainted,

    /// IO should be skipped but not dropped.
    Skip,
}

impl<Io> Future for ConnectionCheckFuture<'_, Io>
where
    Io: AsyncRead + Unpin,
{
    type Output = ConnectionState;

    // this future is only used to get access to Context.
    // It should never return Poll::Pending.
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut buf = [0; 2];
        let mut read_buf = ReadBuf::new(&mut buf);

        let state = match Pin::new(&mut this.io).poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) if !read_buf.filled().is_empty() => ConnectionState::Tainted,

            Poll::Pending => ConnectionState::Live,
            _ => ConnectionState::Skip,
        };

        Poll::Ready(state)
    }
}

struct PooledConnection<Io> {
    conn: ConnectionInnerType<Io>,
    used: Instant,
    created: Instant,
}

pin_project! {
    #[project = CloseConnectionProj]
    struct CloseConnection<Io> {
        io: Io,
        #[pin]
        timeout: Sleep,
    }
}

impl<Io> CloseConnection<Io>
where
    Io: AsyncWrite + Unpin,
{
    fn new(io: Io, timeout: Duration) -> Self {
        CloseConnection {
            io,
            timeout: sleep(timeout),
        }
    }
}

impl<Io> Future for CloseConnection<Io>
where
    Io: AsyncWrite + Unpin,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.project();

        match this.timeout.poll(cx) {
            Poll::Ready(_) => Poll::Ready(()),
            Poll::Pending => Pin::new(this.io).poll_shutdown(cx).map(|_| ()),
        }
    }
}

pub struct Acquired<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    /// authority key for identify connection.
    key: Key,
    /// handle to connection pool.
    inner: ConnectionPoolInner<Io>,
    /// permit for limit concurrent in-flight connection for a Client object.
    permit: OwnedSemaphorePermit,
}

impl<Io: ConnectionIo> Acquired<Io> {
    /// Close the IO.
    pub(super) fn close(&self, conn: ConnectionInnerType<Io>) {
        self.inner.close(conn);
    }

    /// Release IO back into pool.
    pub(super) fn release(&self, conn: ConnectionInnerType<Io>, created: Instant) {
        let Acquired { key, inner, .. } = self;

        inner
            .available
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(VecDeque::new)
            .push_back(PooledConnection {
                conn,
                created,
                used: Instant::now(),
            });

        let _ = &self.permit;
    }
}

#[cfg(test)]
mod test {
    use std::cell::Cell;

    use http::Uri;

    use super::*;

    /// A stream type that always returns pending on async read.
    ///
    /// Mocks an idle TCP stream that is ready to be used for client connections.
    struct TestStream(Rc<Cell<usize>>);

    impl Drop for TestStream {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }

    impl AsyncRead for TestStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for TestStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            unimplemented!()
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            unimplemented!()
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Clone)]
    struct TestPoolConnector {
        generated: Rc<Cell<usize>>,
    }

    impl Service<Connect> for TestPoolConnector {
        type Response = (TestStream, Protocol);
        type Error = ConnectError;
        type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

        actix_service::always_ready!();

        fn call(&self, _: Connect) -> Self::Future {
            self.generated.set(self.generated.get() + 1);
            let generated = self.generated.clone();
            Box::pin(async { Ok((TestStream(generated), Protocol::Http1)) })
        }
    }

    fn release<T>(conn: ConnectionType<T>)
    where
        T: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        match conn {
            ConnectionType::H1(mut conn) => conn.on_release(true),
            ConnectionType::H2(mut conn) => conn.on_release(false),
        }
    }

    #[actix_rt::test]
    async fn test_pool_limit() {
        let connector = TestPoolConnector {
            generated: Rc::new(Cell::new(0)),
        };

        let config = ConnectorConfig {
            limit: 1,
            ..Default::default()
        };

        let pool = super::ConnectionPool::new(connector, config);

        let req = Connect {
            uri: Uri::from_static("http://localhost"),
            addr: None,
        };

        let conn = pool.call(req.clone()).await.unwrap();

        let waiting = Rc::new(Cell::new(true));

        let waiting_clone = waiting.clone();
        actix_rt::spawn(async move {
            actix_rt::time::sleep(Duration::from_millis(100)).await;
            waiting_clone.set(false);
            drop(conn);
        });

        assert!(waiting.get());

        let now = Instant::now();
        let conn = pool.call(req).await.unwrap();

        release(conn);
        assert!(!waiting.get());
        assert!(now.elapsed() >= Duration::from_millis(100));
    }

    #[actix_rt::test]
    async fn test_pool_keep_alive() {
        let generated = Rc::new(Cell::new(0));
        let generated_clone = generated.clone();

        let connector = TestPoolConnector { generated };

        let config = ConnectorConfig {
            conn_keep_alive: Duration::from_secs(1),
            ..Default::default()
        };

        let pool = super::ConnectionPool::new(connector, config);

        let req = Connect {
            uri: Uri::from_static("http://localhost"),
            addr: None,
        };

        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(1, generated_clone.get());
        release(conn);

        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(1, generated_clone.get());
        release(conn);

        actix_rt::time::sleep(Duration::from_millis(1500)).await;
        actix_rt::task::yield_now().await;

        let conn = pool.call(req).await.unwrap();
        // Note: spawned recycle connection is not ran yet.
        // This is tokio current thread runtime specific behavior.
        assert_eq!(2, generated_clone.get());

        // yield task so the old connection is properly dropped.
        actix_rt::task::yield_now().await;
        assert_eq!(1, generated_clone.get());

        release(conn);
    }

    #[actix_rt::test]
    async fn test_pool_lifetime() {
        let generated = Rc::new(Cell::new(0));
        let generated_clone = generated.clone();

        let connector = TestPoolConnector { generated };

        let config = ConnectorConfig {
            conn_lifetime: Duration::from_secs(1),
            ..Default::default()
        };

        let pool = super::ConnectionPool::new(connector, config);

        let req = Connect {
            uri: Uri::from_static("http://localhost"),
            addr: None,
        };

        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(1, generated_clone.get());
        release(conn);

        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(1, generated_clone.get());
        release(conn);

        actix_rt::time::sleep(Duration::from_millis(1500)).await;
        actix_rt::task::yield_now().await;

        let conn = pool.call(req).await.unwrap();
        // Note: spawned recycle connection is not ran yet.
        // This is tokio current thread runtime specific behavior.
        assert_eq!(2, generated_clone.get());

        // yield task so the old connection is properly dropped.
        actix_rt::task::yield_now().await;
        assert_eq!(1, generated_clone.get());

        release(conn);
    }

    #[actix_rt::test]
    async fn test_pool_authority_key() {
        let generated = Rc::new(Cell::new(0));
        let generated_clone = generated.clone();

        let connector = TestPoolConnector { generated };

        let config = ConnectorConfig::default();

        let pool = super::ConnectionPool::new(connector, config);

        let req = Connect {
            uri: Uri::from_static("https://crates.io"),
            addr: None,
        };

        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(1, generated_clone.get());
        release(conn);

        let conn = pool.call(req).await.unwrap();
        assert_eq!(1, generated_clone.get());
        release(conn);

        let req = Connect {
            uri: Uri::from_static("https://google.com"),
            addr: None,
        };

        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(2, generated_clone.get());
        release(conn);
        let conn = pool.call(req).await.unwrap();
        assert_eq!(2, generated_clone.get());
        release(conn);
    }

    #[actix_rt::test]
    async fn test_pool_drop() {
        let generated = Rc::new(Cell::new(0));
        let generated_clone = generated.clone();

        let connector = TestPoolConnector { generated };

        let config = ConnectorConfig::default();

        let pool = Rc::new(super::ConnectionPool::new(connector, config));

        let req = Connect {
            uri: Uri::from_static("https://crates.io"),
            addr: None,
        };

        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(1, generated_clone.get());
        release(conn);

        let req = Connect {
            uri: Uri::from_static("https://google.com"),
            addr: None,
        };
        let conn = pool.call(req.clone()).await.unwrap();
        assert_eq!(2, generated_clone.get());
        release(conn);

        let clone1 = pool.clone();
        let clone2 = clone1.clone();

        drop(clone2);
        for _ in 0..2 {
            actix_rt::task::yield_now().await;
        }
        assert_eq!(2, generated_clone.get());

        drop(clone1);
        for _ in 0..2 {
            actix_rt::task::yield_now().await;
        }
        assert_eq!(2, generated_clone.get());

        drop(pool);
        for _ in 0..2 {
            actix_rt::task::yield_now().await;
        }
        assert_eq!(0, generated_clone.get());
    }
}

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use actix_rt::time::{sleep, Sleep};
use actix_service::Service;
use ahash::AHashMap;
use futures_core::future::LocalBoxFuture;
use http::uri::Authority;
use pin_project::pin_project;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::config::ConnectorConfig;
use super::connection::{ConnectionType, H2Connection, IoConnection};
use super::error::ConnectError;
use super::h2proto::handshake;
use super::Connect;

#[derive(Clone, Copy, PartialEq)]
/// Protocol version
pub enum Protocol {
    Http1,
    Http2,
}

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(crate) struct Key {
    authority: Authority,
}

impl From<Authority> for Key {
    fn from(authority: Authority) -> Key {
        Key { authority }
    }
}

/// Connections pool for reuse Io type for certain [`http::uri::Authority`] as key
pub(crate) struct ConnectionPool<S, Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    connector: Rc<S>,
    inner: ConnectionPoolInner<Io>,
}

/// wrapper type for check the ref count of Rc.
struct ConnectionPoolInner<Io>(Rc<ConnectionPoolInnerPriv<Io>>)
where
    Io: AsyncWrite + Unpin + 'static;

impl<Io> ConnectionPoolInner<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    /// spawn a async for graceful shutdown h1 Io type with a timeout.
    fn close(&self, conn: ConnectionType<Io>) {
        if let Some(timeout) = self.config.disconnect_timeout {
            if let ConnectionType::H1(io) = conn {
                actix_rt::spawn(CloseConnection::new(io, timeout));
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
        &*self.0
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

struct ConnectionPoolInnerPriv<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    config: ConnectorConfig,
    available: RefCell<AHashMap<Key, VecDeque<PooledConnection<Io>>>>,
    permits: Arc<Semaphore>,
}

impl<S, Io> ConnectionPool<S, Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    /// construct a new connection pool.
    ///
    /// [`super::config::ConnectorConfig`]'s `limit` is used as the max permits allowed
    /// for on flight connections.
    ///
    /// The pool can only have equal to `limit` amount of requests spawning/using Io type
    /// concurrently.
    ///
    /// Any requests beyond limit would be wait in fifo order and get notified in async
    /// manner by [`tokio::sync::Semaphore`]
    pub(crate) fn new(connector: S, config: ConnectorConfig) -> Self {
        let permits = Arc::new(Semaphore::new(config.limit));
        let available = RefCell::new(AHashMap::default());
        let connector = Rc::new(connector);

        let inner = ConnectionPoolInner(Rc::new(ConnectionPoolInnerPriv {
            config,
            available,
            permits,
        }));

        Self { connector, inner }
    }
}

impl<S, Io> Clone for ConnectionPool<S, Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<S, Io> Service<Connect> for ConnectionPool<S, Io>
where
    S: Service<Connect, Response = (Io, Protocol), Error = ConnectError> + 'static,
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Response = IoConnection<Io>;
    type Error = ConnectError;
    type Future = LocalBoxFuture<'static, Result<IoConnection<Io>, ConnectError>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.connector.poll_ready(cx)
    }

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
            let permit = inner
                .permits
                .clone()
                .acquire_owned()
                .await
                // TODO: use specific error for semaphore acquire error
                .map_err(|_| ConnectError::NoRecords)?;

            // check if there is idle connection for given key.
            let mut map = inner.available.borrow_mut();

            let mut conn = None;
            if let Some(conns) = map.get_mut(&key) {
                let now = Instant::now();
                while let Some(mut c) = conns.pop_front() {
                    // check the lifetime and drop connection that live for too long.
                    if (now - c.used) > inner.config.conn_keep_alive
                        || (now - c.created) > inner.config.conn_lifetime
                    {
                        inner.close(c.conn);
                    // check if the connection is still usable.
                    } else {
                        if let ConnectionType::H1(ref mut io) = c.conn {
                            let check = ConnectionCheckFuture { io };
                            match check.await {
                                ConnectionState::Break => {
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

            // drop map early to end the borrow_mut of RefCell.
            drop(map);

            // construct acquired. It's used to put Io type back to pool/ close the Io type.
            // permit is carried with the whole lifecycle of Acquired.
            let acquired = Some(Acquired { key, inner, permit });

            // match the connection and spawn new one if did not get anything.
            match conn {
                Some(conn) => Ok(IoConnection::new(conn.conn, conn.created, acquired)),
                None => {
                    let (io, proto) = connector.call(req).await?;

                    if proto == Protocol::Http1 {
                        Ok(IoConnection::new(
                            ConnectionType::H1(io),
                            Instant::now(),
                            acquired,
                        ))
                    } else {
                        let config = &acquired.as_ref().unwrap().inner.config;
                        let (sender, connection) = handshake(io, config).await?;
                        Ok(IoConnection::new(
                            ConnectionType::H2(H2Connection::new(sender, connection)),
                            Instant::now(),
                            acquired,
                        ))
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
    Live,
    Break,
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
            // io is pending and new data would wake up it.
            Poll::Pending => ConnectionState::Live,
            // io have data inside. drop it.
            Poll::Ready(Ok(())) if !read_buf.filled().is_empty() => {
                ConnectionState::Break
            }
            // otherwise skip to next.
            _ => ConnectionState::Skip,
        };

        Poll::Ready(state)
    }
}

struct PooledConnection<Io> {
    conn: ConnectionType<Io>,
    used: Instant,
    created: Instant,
}

#[pin_project]
struct CloseConnection<Io> {
    io: Io,
    #[pin]
    timeout: Sleep,
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

pub(crate) struct Acquired<Io>
where
    Io: AsyncWrite + Unpin + 'static,
{
    key: Key,
    inner: ConnectionPoolInner<Io>,
    permit: OwnedSemaphorePermit,
}

impl<Io> Acquired<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    // close the Io type.
    pub(crate) fn close(&mut self, conn: IoConnection<Io>) {
        let (conn, _) = conn.into_inner();
        self.inner.close(conn);
    }

    // put the Io type back to pool.
    pub(crate) fn release(&mut self, conn: IoConnection<Io>) {
        let (io, created) = conn.into_inner();
        let Acquired { key, inner, .. } = self;
        inner
            .available
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(VecDeque::new)
            .push_back(PooledConnection {
                conn: io,
                created,
                used: Instant::now(),
            });

        // a no op bind. used to stop clippy warning without adding allow attribute.
        let _permit = &mut self.permit;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::cell::Cell;
    use std::io;

    use http::Uri;

    use crate::client::connection::IoConnection;

    // A stream type always return pending on async read.
    // mock a usable tcp stream that ready to be used as client
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

        fn poll_flush(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            unimplemented!()
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct TestPoolConnector {
        generated: Rc<Cell<usize>>,
    }

    impl Service<Connect> for TestPoolConnector {
        type Response = (TestStream, Protocol);
        type Error = ConnectError;
        type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

        fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            unimplemented!("poll_ready is not used in test")
        }

        fn call(&self, _: Connect) -> Self::Future {
            self.generated.set(self.generated.get() + 1);
            let generated = self.generated.clone();
            Box::pin(async { Ok((TestStream(generated), Protocol::Http1)) })
        }
    }

    fn release<T>(conn: IoConnection<T>)
    where
        T: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let (conn, created, mut acquired) = conn.into_parts();
        acquired.release(IoConnection::new(conn, created, None));
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
        assert_eq!(2, generated_clone.get());
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
        assert_eq!(2, generated_clone.get());
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

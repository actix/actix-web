use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::time::{delay_for, Delay};
use actix_service::Service;
use actix_utils::{oneshot, task::LocalWaker};
use bytes::Bytes;
use futures_util::future::{poll_fn, FutureExt, LocalBoxFuture};
use fxhash::FxHashMap;
use h2::client::{Connection, SendRequest};
use http::uri::Authority;
use indexmap::IndexSet;
use pin_project::pin_project;
use slab::Slab;

use super::config::ConnectorConfig;
use super::connection::{ConnectionType, IoConnection};
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

/// Connections pool
pub(crate) struct ConnectionPool<T, Io: 'static>(Rc<RefCell<T>>, Rc<RefCell<Inner<Io>>>);

impl<T, Io> ConnectionPool<T, Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
    T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
        + 'static,
{
    pub(crate) fn new(connector: T, config: ConnectorConfig) -> Self {
        let connector_rc = Rc::new(RefCell::new(connector));
        let inner_rc = Rc::new(RefCell::new(Inner {
            config,
            acquired: 0,
            waiters: Slab::new(),
            waiters_queue: IndexSet::new(),
            available: FxHashMap::default(),
            waker: LocalWaker::new(),
        }));

        // start support future
        actix_rt::spawn(ConnectorPoolSupport {
            connector: Rc::clone(&connector_rc),
            inner: Rc::clone(&inner_rc),
        });

        ConnectionPool(connector_rc, inner_rc)
    }
}

impl<T, Io> Clone for ConnectionPool<T, Io>
where
    Io: 'static,
{
    fn clone(&self) -> Self {
        ConnectionPool(self.0.clone(), self.1.clone())
    }
}

impl<T, Io> Drop for ConnectionPool<T, Io> {
    fn drop(&mut self) {
        // wake up the ConnectorPoolSupport when dropping so it can exit properly.
        self.1.borrow().waker.wake();
    }
}

impl<T, Io> Service for ConnectionPool<T, Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
    T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
        + 'static,
{
    type Request = Connect;
    type Response = IoConnection<Io>;
    type Error = ConnectError;
    type Future = LocalBoxFuture<'static, Result<IoConnection<Io>, ConnectError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: Connect) -> Self::Future {
        let mut connector = self.0.clone();
        let inner = self.1.clone();

        let fut = async move {
            let key = if let Some(authority) = req.uri.authority() {
                authority.clone().into()
            } else {
                return Err(ConnectError::Unresolved);
            };

            // acquire connection
            match poll_fn(|cx| Poll::Ready(inner.borrow_mut().acquire(&key, cx))).await {
                Acquire::Acquired(io, created) => {
                    // use existing connection
                    Ok(IoConnection::new(
                        io,
                        created,
                        Some(Acquired(key, Some(inner))),
                    ))
                }
                Acquire::Available => {
                    // open tcp connection
                    let (io, proto) = connector.call(req).await?;

                    let config = inner.borrow().config.clone();

                    let guard = OpenGuard::new(key, inner);

                    if proto == Protocol::Http1 {
                        Ok(IoConnection::new(
                            ConnectionType::H1(io),
                            Instant::now(),
                            Some(guard.consume()),
                        ))
                    } else {
                        let (snd, connection) = handshake(io, &config).await?;
                        actix_rt::spawn(connection.map(|_| ()));
                        Ok(IoConnection::new(
                            ConnectionType::H2(snd),
                            Instant::now(),
                            Some(guard.consume()),
                        ))
                    }
                }
                _ => {
                    // connection is not available, wait
                    let (rx, token) = inner.borrow_mut().wait_for(req);

                    let guard = WaiterGuard::new(key, token, inner);
                    let res = match rx.await {
                        Err(_) => Err(ConnectError::Disconnected),
                        Ok(res) => res,
                    };
                    guard.consume();
                    res
                }
            }
        };

        fut.boxed_local()
    }
}

struct WaiterGuard<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    key: Key,
    token: usize,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<Io> WaiterGuard<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    fn new(key: Key, token: usize, inner: Rc<RefCell<Inner<Io>>>) -> Self {
        Self {
            key,
            token,
            inner: Some(inner),
        }
    }

    fn consume(mut self) {
        let _ = self.inner.take();
    }
}

impl<Io> Drop for WaiterGuard<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    fn drop(&mut self) {
        if let Some(i) = self.inner.take() {
            let mut inner = i.as_ref().borrow_mut();
            inner.release_waiter(&self.key, self.token);
            inner.check_availability();
        }
    }
}

struct OpenGuard<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    key: Key,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<Io> OpenGuard<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    fn new(key: Key, inner: Rc<RefCell<Inner<Io>>>) -> Self {
        Self {
            key,
            inner: Some(inner),
        }
    }

    fn consume(mut self) -> Acquired<Io> {
        Acquired(self.key.clone(), self.inner.take())
    }
}

impl<Io> Drop for OpenGuard<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    fn drop(&mut self) {
        if let Some(i) = self.inner.take() {
            let mut inner = i.as_ref().borrow_mut();
            inner.release();
            inner.check_availability();
        }
    }
}

enum Acquire<T> {
    Acquired(ConnectionType<T>, Instant),
    Available,
    NotAvailable,
}

struct AvailableConnection<Io> {
    io: ConnectionType<Io>,
    used: Instant,
    created: Instant,
}

pub(crate) struct Inner<Io> {
    config: ConnectorConfig,
    acquired: usize,
    available: FxHashMap<Key, VecDeque<AvailableConnection<Io>>>,
    waiters: Slab<
        Option<(
            Connect,
            oneshot::Sender<Result<IoConnection<Io>, ConnectError>>,
        )>,
    >,
    waiters_queue: IndexSet<(Key, usize)>,
    waker: LocalWaker,
}

impl<Io> Inner<Io> {
    fn reserve(&mut self) {
        self.acquired += 1;
    }

    fn release(&mut self) {
        self.acquired -= 1;
    }

    fn release_waiter(&mut self, key: &Key, token: usize) {
        self.waiters.remove(token);
        let _ = self.waiters_queue.shift_remove(&(key.clone(), token));
    }
}

impl<Io> Inner<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    /// connection is not available, wait
    fn wait_for(
        &mut self,
        connect: Connect,
    ) -> (
        oneshot::Receiver<Result<IoConnection<Io>, ConnectError>>,
        usize,
    ) {
        let (tx, rx) = oneshot::channel();

        let key: Key = connect.uri.authority().unwrap().clone().into();
        let entry = self.waiters.vacant_entry();
        let token = entry.key();
        entry.insert(Some((connect, tx)));
        assert!(self.waiters_queue.insert((key, token)));

        (rx, token)
    }

    fn acquire(&mut self, key: &Key, cx: &mut Context<'_>) -> Acquire<Io> {
        // check limits
        if self.config.limit > 0 && self.acquired >= self.config.limit {
            return Acquire::NotAvailable;
        }

        self.reserve();

        // check if open connection is available
        // cleanup stale connections at the same time
        if let Some(ref mut connections) = self.available.get_mut(key) {
            let now = Instant::now();
            while let Some(conn) = connections.pop_back() {
                // check if it still usable
                if (now - conn.used) > self.config.conn_keep_alive
                    || (now - conn.created) > self.config.conn_lifetime
                {
                    if let Some(timeout) = self.config.disconnect_timeout {
                        if let ConnectionType::H1(io) = conn.io {
                            actix_rt::spawn(CloseConnection::new(io, timeout))
                        }
                    }
                } else {
                    let mut io = conn.io;
                    let mut buf = [0; 2];
                    if let ConnectionType::H1(ref mut s) = io {
                        match Pin::new(s).poll_read(cx, &mut buf) {
                            Poll::Pending => (),
                            Poll::Ready(Ok(n)) if n > 0 => {
                                if let Some(timeout) = self.config.disconnect_timeout {
                                    if let ConnectionType::H1(io) = io {
                                        actix_rt::spawn(CloseConnection::new(
                                            io, timeout,
                                        ))
                                    }
                                }
                                continue;
                            }
                            _ => continue,
                        }
                    }
                    return Acquire::Acquired(io, conn.created);
                }
            }
        }
        Acquire::Available
    }

    fn release_conn(&mut self, key: &Key, io: ConnectionType<Io>, created: Instant) {
        self.acquired -= 1;
        self.available
            .entry(key.clone())
            .or_insert_with(VecDeque::new)
            .push_back(AvailableConnection {
                io,
                created,
                used: Instant::now(),
            });
        self.check_availability();
    }

    fn release_close(&mut self, io: ConnectionType<Io>) {
        self.acquired -= 1;
        if let Some(timeout) = self.config.disconnect_timeout {
            if let ConnectionType::H1(io) = io {
                actix_rt::spawn(CloseConnection::new(io, timeout))
            }
        }
        self.check_availability();
    }

    fn check_availability(&self) {
        if !self.waiters_queue.is_empty() && self.acquired < self.config.limit {
            self.waker.wake();
        }
    }
}

struct CloseConnection<T> {
    io: T,
    timeout: Delay,
}

impl<T> CloseConnection<T>
where
    T: AsyncWrite + Unpin,
{
    fn new(io: T, timeout: Duration) -> Self {
        CloseConnection {
            io,
            timeout: delay_for(timeout),
        }
    }
}

impl<T> Future for CloseConnection<T>
where
    T: AsyncWrite + Unpin,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();

        match Pin::new(&mut this.timeout).poll(cx) {
            Poll::Ready(_) => Poll::Ready(()),
            Poll::Pending => match Pin::new(&mut this.io).poll_shutdown(cx) {
                Poll::Ready(_) => Poll::Ready(()),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

#[pin_project]
struct ConnectorPoolSupport<T, Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    connector: T,
    inner: Rc<RefCell<Inner<Io>>>,
}

impl<T, Io> Future for ConnectorPoolSupport<T, Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
    T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>,
    T::Future: 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if Rc::strong_count(this.inner) == 1 {
            // If we are last copy of Inner<Io> it means the ConnectionPool is already gone
            // and we are safe to exit.
            return Poll::Ready(());
        }

        let mut inner = this.inner.borrow_mut();
        inner.waker.register(cx.waker());

        // check waiters
        loop {
            let (key, token) = {
                if let Some((key, token)) = inner.waiters_queue.get_index(0) {
                    (key.clone(), *token)
                } else {
                    break;
                }
            };
            if inner.waiters.get(token).unwrap().is_none() {
                continue;
            }

            match inner.acquire(&key, cx) {
                Acquire::NotAvailable => break,
                Acquire::Acquired(io, created) => {
                    let tx = inner.waiters.get_mut(token).unwrap().take().unwrap().1;
                    if let Err(conn) = tx.send(Ok(IoConnection::new(
                        io,
                        created,
                        Some(Acquired(key.clone(), Some(this.inner.clone()))),
                    ))) {
                        let (io, created) = conn.unwrap().into_inner();
                        inner.release_conn(&key, io, created);
                    }
                }
                Acquire::Available => {
                    let (connect, tx) =
                        inner.waiters.get_mut(token).unwrap().take().unwrap();
                    OpenWaitingConnection::spawn(
                        key.clone(),
                        tx,
                        this.inner.clone(),
                        this.connector.call(connect),
                        inner.config.clone(),
                    );
                }
            }
            let _ = inner.waiters_queue.swap_remove_index(0);
        }

        Poll::Pending
    }
}

#[pin_project::pin_project(PinnedDrop)]
struct OpenWaitingConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    #[pin]
    fut: F,
    key: Key,
    h2: Option<
        LocalBoxFuture<
            'static,
            Result<(SendRequest<Bytes>, Connection<Io, Bytes>), h2::Error>,
        >,
    >,
    rx: Option<oneshot::Sender<Result<IoConnection<Io>, ConnectError>>>,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
    config: ConnectorConfig,
}

impl<F, Io> OpenWaitingConnection<F, Io>
where
    F: Future<Output = Result<(Io, Protocol), ConnectError>> + 'static,
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    fn spawn(
        key: Key,
        rx: oneshot::Sender<Result<IoConnection<Io>, ConnectError>>,
        inner: Rc<RefCell<Inner<Io>>>,
        fut: F,
        config: ConnectorConfig,
    ) {
        actix_rt::spawn(OpenWaitingConnection {
            key,
            fut,
            h2: None,
            rx: Some(rx),
            inner: Some(inner),
            config,
        })
    }
}

#[pin_project::pinned_drop]
impl<F, Io> PinnedDrop for OpenWaitingConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + 'static,
{
    fn drop(self: Pin<&mut Self>) {
        if let Some(inner) = self.project().inner.take() {
            let mut inner = inner.as_ref().borrow_mut();
            inner.release();
            inner.check_availability();
        }
    }
}

impl<F, Io> Future for OpenWaitingConnection<F, Io>
where
    F: Future<Output = Result<(Io, Protocol), ConnectError>>,
    Io: AsyncRead + AsyncWrite + Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        if let Some(ref mut h2) = this.h2 {
            return match Pin::new(h2).poll(cx) {
                Poll::Ready(Ok((snd, connection))) => {
                    actix_rt::spawn(connection.map(|_| ()));
                    let rx = this.rx.take().unwrap();
                    let _ = rx.send(Ok(IoConnection::new(
                        ConnectionType::H2(snd),
                        Instant::now(),
                        Some(Acquired(this.key.clone(), this.inner.take())),
                    )));
                    Poll::Ready(())
                }
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(err)) => {
                    let _ = this.inner.take();
                    if let Some(rx) = this.rx.take() {
                        let _ = rx.send(Err(ConnectError::H2(err)));
                    }
                    Poll::Ready(())
                }
            };
        }

        match this.fut.poll(cx) {
            Poll::Ready(Err(err)) => {
                let _ = this.inner.take();
                if let Some(rx) = this.rx.take() {
                    let _ = rx.send(Err(err));
                }
                Poll::Ready(())
            }
            Poll::Ready(Ok((io, proto))) => {
                if proto == Protocol::Http1 {
                    let rx = this.rx.take().unwrap();
                    let _ = rx.send(Ok(IoConnection::new(
                        ConnectionType::H1(io),
                        Instant::now(),
                        Some(Acquired(this.key.clone(), this.inner.take())),
                    )));
                    Poll::Ready(())
                } else {
                    *this.h2 = Some(handshake(io, this.config).boxed_local());
                    self.poll(cx)
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

pub(crate) struct Acquired<T>(Key, Option<Rc<RefCell<Inner<T>>>>);

impl<T> Acquired<T>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    pub(crate) fn close(&mut self, conn: IoConnection<T>) {
        if let Some(inner) = self.1.take() {
            let (io, _) = conn.into_inner();
            inner.as_ref().borrow_mut().release_close(io);
        }
    }
    pub(crate) fn release(&mut self, conn: IoConnection<T>) {
        if let Some(inner) = self.1.take() {
            let (io, created) = conn.into_inner();
            inner
                .as_ref()
                .borrow_mut()
                .release_conn(&self.0, io, created);
        }
    }
}

impl<T> Drop for Acquired<T> {
    fn drop(&mut self) {
        if let Some(inner) = self.1.take() {
            inner.as_ref().borrow_mut().release();
        }
    }
}

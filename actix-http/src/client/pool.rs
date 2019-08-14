use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

use actix_codec::{AsyncRead, AsyncWrite};
use actix_service::Service;
use bytes::Bytes;
use futures::future::{err, ok, Either, FutureResult};
use futures::task::AtomicTask;
use futures::unsync::oneshot;
use futures::{Async, Future, Poll};
use h2::client::{handshake, Handshake};
use hashbrown::HashMap;
use http::uri::Authority;
use indexmap::IndexSet;
use slab::Slab;
use tokio_timer::{sleep, Delay};

use super::connection::{ConnectionType, IoConnection};
use super::error::ConnectError;
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
pub(crate) struct ConnectionPool<T, Io: AsyncRead + AsyncWrite + 'static>(
    T,
    Rc<RefCell<Inner<Io>>>,
);

impl<T, Io> ConnectionPool<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
    T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
        + Clone
        + 'static,
{
    pub(crate) fn new(
        connector: T,
        conn_lifetime: Duration,
        conn_keep_alive: Duration,
        disconnect_timeout: Option<Duration>,
        limit: usize,
    ) -> Self {
        ConnectionPool(
            connector,
            Rc::new(RefCell::new(Inner {
                conn_lifetime,
                conn_keep_alive,
                disconnect_timeout,
                limit,
                acquired: 0,
                waiters: Slab::new(),
                waiters_queue: IndexSet::new(),
                available: HashMap::new(),
                task: None,
            })),
        )
    }
}

impl<T, Io> Clone for ConnectionPool<T, Io>
where
    T: Clone,
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn clone(&self) -> Self {
        ConnectionPool(self.0.clone(), self.1.clone())
    }
}

impl<T, Io> Service for ConnectionPool<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
    T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
        + Clone
        + 'static,
{
    type Request = Connect;
    type Response = IoConnection<Io>;
    type Error = ConnectError;
    type Future = Either<
        FutureResult<Self::Response, Self::Error>,
        Either<WaitForConnection<Io>, OpenConnection<T::Future, Io>>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.0.poll_ready()
    }

    fn call(&mut self, req: Connect) -> Self::Future {
        let key = if let Some(authority) = req.uri.authority_part() {
            authority.clone().into()
        } else {
            return Either::A(err(ConnectError::Unresolverd));
        };

        // acquire connection
        match self.1.as_ref().borrow_mut().acquire(&key) {
            Acquire::Acquired(io, created) => {
                // use existing connection
                return Either::A(ok(IoConnection::new(
                    io,
                    created,
                    Some(Acquired(key, Some(self.1.clone()))),
                )));
            }
            Acquire::Available => {
                // open new connection
                return Either::B(Either::B(OpenConnection::new(
                    key,
                    self.1.clone(),
                    self.0.call(req),
                )));
            }
            _ => (),
        }

        // connection is not available, wait
        let (rx, token, support) = self.1.as_ref().borrow_mut().wait_for(req);

        // start support future
        if !support {
            self.1.as_ref().borrow_mut().task = Some(AtomicTask::new());
            tokio_current_thread::spawn(ConnectorPoolSupport {
                connector: self.0.clone(),
                inner: self.1.clone(),
            })
        }

        Either::B(Either::A(WaitForConnection {
            rx,
            key,
            token,
            inner: Some(self.1.clone()),
        }))
    }
}

#[doc(hidden)]
pub struct WaitForConnection<Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    key: Key,
    token: usize,
    rx: oneshot::Receiver<Result<IoConnection<Io>, ConnectError>>,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<Io> Drop for WaitForConnection<Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn drop(&mut self) {
        if let Some(i) = self.inner.take() {
            let mut inner = i.as_ref().borrow_mut();
            inner.release_waiter(&self.key, self.token);
            inner.check_availibility();
        }
    }
}

impl<Io> Future for WaitForConnection<Io>
where
    Io: AsyncRead + AsyncWrite,
{
    type Item = IoConnection<Io>;
    type Error = ConnectError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.rx.poll() {
            Ok(Async::Ready(item)) => match item {
                Err(err) => Err(err),
                Ok(conn) => {
                    let _ = self.inner.take();
                    Ok(Async::Ready(conn))
                }
            },
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(_) => {
                let _ = self.inner.take();
                Err(ConnectError::Disconnected)
            }
        }
    }
}

#[doc(hidden)]
pub struct OpenConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fut: F,
    key: Key,
    h2: Option<Handshake<Io, Bytes>>,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<F, Io> OpenConnection<F, Io>
where
    F: Future<Item = (Io, Protocol), Error = ConnectError>,
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn new(key: Key, inner: Rc<RefCell<Inner<Io>>>, fut: F) -> Self {
        OpenConnection {
            key,
            fut,
            inner: Some(inner),
            h2: None,
        }
    }
}

impl<F, Io> Drop for OpenConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            let mut inner = inner.as_ref().borrow_mut();
            inner.release();
            inner.check_availibility();
        }
    }
}

impl<F, Io> Future for OpenConnection<F, Io>
where
    F: Future<Item = (Io, Protocol), Error = ConnectError>,
    Io: AsyncRead + AsyncWrite,
{
    type Item = IoConnection<Io>;
    type Error = ConnectError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut h2) = self.h2 {
            return match h2.poll() {
                Ok(Async::Ready((snd, connection))) => {
                    tokio_current_thread::spawn(connection.map_err(|_| ()));
                    Ok(Async::Ready(IoConnection::new(
                        ConnectionType::H2(snd),
                        Instant::now(),
                        Some(Acquired(self.key.clone(), self.inner.take())),
                    )))
                }
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(e) => Err(e.into()),
            };
        }

        match self.fut.poll() {
            Err(err) => Err(err),
            Ok(Async::Ready((io, proto))) => {
                if proto == Protocol::Http1 {
                    Ok(Async::Ready(IoConnection::new(
                        ConnectionType::H1(io),
                        Instant::now(),
                        Some(Acquired(self.key.clone(), self.inner.take())),
                    )))
                } else {
                    self.h2 = Some(handshake(io));
                    self.poll()
                }
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
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
    conn_lifetime: Duration,
    conn_keep_alive: Duration,
    disconnect_timeout: Option<Duration>,
    limit: usize,
    acquired: usize,
    available: HashMap<Key, VecDeque<AvailableConnection<Io>>>,
    waiters: Slab<
        Option<(
            Connect,
            oneshot::Sender<Result<IoConnection<Io>, ConnectError>>,
        )>,
    >,
    waiters_queue: IndexSet<(Key, usize)>,
    task: Option<AtomicTask>,
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
        self.waiters_queue.remove(&(key.clone(), token));
    }
}

impl<Io> Inner<Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    /// connection is not available, wait
    fn wait_for(
        &mut self,
        connect: Connect,
    ) -> (
        oneshot::Receiver<Result<IoConnection<Io>, ConnectError>>,
        usize,
        bool,
    ) {
        let (tx, rx) = oneshot::channel();

        let key: Key = connect.uri.authority_part().unwrap().clone().into();
        let entry = self.waiters.vacant_entry();
        let token = entry.key();
        entry.insert(Some((connect, tx)));
        assert!(self.waiters_queue.insert((key, token)));

        (rx, token, self.task.is_some())
    }

    fn acquire(&mut self, key: &Key) -> Acquire<Io> {
        // check limits
        if self.limit > 0 && self.acquired >= self.limit {
            return Acquire::NotAvailable;
        }

        self.reserve();

        // check if open connection is available
        // cleanup stale connections at the same time
        if let Some(ref mut connections) = self.available.get_mut(key) {
            let now = Instant::now();
            while let Some(conn) = connections.pop_back() {
                // check if it still usable
                if (now - conn.used) > self.conn_keep_alive
                    || (now - conn.created) > self.conn_lifetime
                {
                    if let Some(timeout) = self.disconnect_timeout {
                        if let ConnectionType::H1(io) = conn.io {
                            tokio_current_thread::spawn(CloseConnection::new(
                                io, timeout,
                            ))
                        }
                    }
                } else {
                    let mut io = conn.io;
                    let mut buf = [0; 2];
                    if let ConnectionType::H1(ref mut s) = io {
                        match s.read(&mut buf) {
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => (),
                            Ok(n) if n > 0 => {
                                if let Some(timeout) = self.disconnect_timeout {
                                    if let ConnectionType::H1(io) = io {
                                        tokio_current_thread::spawn(
                                            CloseConnection::new(io, timeout),
                                        )
                                    }
                                }
                                continue;
                            }
                            Ok(_) | Err(_) => continue,
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
        self.check_availibility();
    }

    fn release_close(&mut self, io: ConnectionType<Io>) {
        self.acquired -= 1;
        if let Some(timeout) = self.disconnect_timeout {
            if let ConnectionType::H1(io) = io {
                tokio_current_thread::spawn(CloseConnection::new(io, timeout))
            }
        }
        self.check_availibility();
    }

    fn check_availibility(&self) {
        if !self.waiters_queue.is_empty() && self.acquired < self.limit {
            if let Some(t) = self.task.as_ref() {
                t.notify()
            }
        }
    }
}

struct CloseConnection<T> {
    io: T,
    timeout: Delay,
}

impl<T> CloseConnection<T>
where
    T: AsyncWrite,
{
    fn new(io: T, timeout: Duration) -> Self {
        CloseConnection {
            io,
            timeout: sleep(timeout),
        }
    }
}

impl<T> Future for CloseConnection<T>
where
    T: AsyncWrite,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        match self.timeout.poll() {
            Ok(Async::Ready(_)) | Err(_) => Ok(Async::Ready(())),
            Ok(Async::NotReady) => match self.io.shutdown() {
                Ok(Async::Ready(_)) | Err(_) => Ok(Async::Ready(())),
                Ok(Async::NotReady) => Ok(Async::NotReady),
            },
        }
    }
}

struct ConnectorPoolSupport<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    connector: T,
    inner: Rc<RefCell<Inner<Io>>>,
}

impl<T, Io> Future for ConnectorPoolSupport<T, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
    T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>,
    T::Future: 'static,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut inner = self.inner.as_ref().borrow_mut();
        inner.task.as_ref().unwrap().register();

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

            match inner.acquire(&key) {
                Acquire::NotAvailable => break,
                Acquire::Acquired(io, created) => {
                    let tx = inner.waiters.get_mut(token).unwrap().take().unwrap().1;
                    if let Err(conn) = tx.send(Ok(IoConnection::new(
                        io,
                        created,
                        Some(Acquired(key.clone(), Some(self.inner.clone()))),
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
                        self.inner.clone(),
                        self.connector.call(connect),
                    );
                }
            }
            let _ = inner.waiters_queue.swap_remove_index(0);
        }

        Ok(Async::NotReady)
    }
}

struct OpenWaitingConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fut: F,
    key: Key,
    h2: Option<Handshake<Io, Bytes>>,
    rx: Option<oneshot::Sender<Result<IoConnection<Io>, ConnectError>>>,
    inner: Option<Rc<RefCell<Inner<Io>>>>,
}

impl<F, Io> OpenWaitingConnection<F, Io>
where
    F: Future<Item = (Io, Protocol), Error = ConnectError> + 'static,
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn spawn(
        key: Key,
        rx: oneshot::Sender<Result<IoConnection<Io>, ConnectError>>,
        inner: Rc<RefCell<Inner<Io>>>,
        fut: F,
    ) {
        tokio_current_thread::spawn(OpenWaitingConnection {
            key,
            fut,
            h2: None,
            rx: Some(rx),
            inner: Some(inner),
        })
    }
}

impl<F, Io> Drop for OpenWaitingConnection<F, Io>
where
    Io: AsyncRead + AsyncWrite + 'static,
{
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            let mut inner = inner.as_ref().borrow_mut();
            inner.release();
            inner.check_availibility();
        }
    }
}

impl<F, Io> Future for OpenWaitingConnection<F, Io>
where
    F: Future<Item = (Io, Protocol), Error = ConnectError>,
    Io: AsyncRead + AsyncWrite,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut h2) = self.h2 {
            return match h2.poll() {
                Ok(Async::Ready((snd, connection))) => {
                    tokio_current_thread::spawn(connection.map_err(|_| ()));
                    let rx = self.rx.take().unwrap();
                    let _ = rx.send(Ok(IoConnection::new(
                        ConnectionType::H2(snd),
                        Instant::now(),
                        Some(Acquired(self.key.clone(), self.inner.take())),
                    )));
                    Ok(Async::Ready(()))
                }
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(err) => {
                    let _ = self.inner.take();
                    if let Some(rx) = self.rx.take() {
                        let _ = rx.send(Err(ConnectError::H2(err)));
                    }
                    Err(())
                }
            };
        }

        match self.fut.poll() {
            Err(err) => {
                let _ = self.inner.take();
                if let Some(rx) = self.rx.take() {
                    let _ = rx.send(Err(err));
                }
                Err(())
            }
            Ok(Async::Ready((io, proto))) => {
                if proto == Protocol::Http1 {
                    let rx = self.rx.take().unwrap();
                    let _ = rx.send(Ok(IoConnection::new(
                        ConnectionType::H1(io),
                        Instant::now(),
                        Some(Acquired(self.key.clone(), self.inner.take())),
                    )));
                    Ok(Async::Ready(()))
                } else {
                    self.h2 = Some(handshake(io));
                    self.poll()
                }
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
        }
    }
}

pub(crate) struct Acquired<T>(Key, Option<Rc<RefCell<Inner<T>>>>);

impl<T> Acquired<T>
where
    T: AsyncRead + AsyncWrite + 'static,
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

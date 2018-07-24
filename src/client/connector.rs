use std::collections::{HashMap, VecDeque};
use std::net::Shutdown;
use std::time::{Duration, Instant};
use std::{fmt, io, mem, time};

use actix::resolver::{Connect as ResolveConnect, Resolver, ResolverError};
use actix::{
    fut, Actor, ActorFuture, ActorResponse, Addr, AsyncContext, Context,
    ContextFutureSpawner, Handler, Message, Recipient, StreamHandler, Supervised,
    SystemService, WrapFuture,
};

use futures::sync::{mpsc, oneshot};
use futures::{Async, Future, Poll};
use http::{Error as HttpError, HttpTryFrom, Uri};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_timer::Delay;

#[cfg(feature = "alpn")]
use openssl::ssl::{Error as OpensslError, SslConnector, SslMethod};
#[cfg(feature = "alpn")]
use tokio_openssl::SslConnectorExt;

#[cfg(all(feature = "tls", not(feature = "alpn")))]
use native_tls::{Error as TlsError, TlsConnector};
#[cfg(all(feature = "tls", not(feature = "alpn")))]
use tokio_tls::TlsConnectorExt;

use server::IoStream;
use {HAS_OPENSSL, HAS_TLS};

/// Client connector usage stats
#[derive(Default, Message)]
pub struct ClientConnectorStats {
    /// Number of waited-on connections
    pub waits: usize,
    /// Size of the wait queue
    pub wait_queue: usize,
    /// Number of reused connections
    pub reused: usize,
    /// Number of opened connections
    pub opened: usize,
    /// Number of closed connections
    pub closed: usize,
    /// Number of connections with errors
    pub errors: usize,
    /// Number of connection timeouts
    pub timeouts: usize,
}

#[derive(Debug)]
/// `Connect` type represents a message that can be sent to
/// `ClientConnector` with a connection request.
pub struct Connect {
    pub(crate) uri: Uri,
    pub(crate) wait_timeout: Duration,
    pub(crate) conn_timeout: Duration,
}

impl Connect {
    /// Create `Connect` message for specified `Uri`
    pub fn new<U>(uri: U) -> Result<Connect, HttpError>
    where
        Uri: HttpTryFrom<U>,
    {
        Ok(Connect {
            uri: Uri::try_from(uri).map_err(|e| e.into())?,
            wait_timeout: Duration::from_secs(5),
            conn_timeout: Duration::from_secs(1),
        })
    }

    /// Connection timeout, i.e. max time to connect to remote host.
    /// Set to 1 second by default.
    pub fn conn_timeout(mut self, timeout: Duration) -> Self {
        self.conn_timeout = timeout;
        self
    }

    /// If connection pool limits are enabled, wait time indicates
    /// max time to wait for a connection to become available.
    /// Set to 5 seconds by default.
    pub fn wait_timeout(mut self, timeout: Duration) -> Self {
        self.wait_timeout = timeout;
        self
    }
}

impl Message for Connect {
    type Result = Result<Connection, ClientConnectorError>;
}

/// Pause connection process for `ClientConnector`
///
/// All connect requests enter wait state during connector pause.
pub struct Pause {
    time: Option<Duration>,
}

impl Pause {
    /// Create message with pause duration parameter
    pub fn new(time: Duration) -> Pause {
        Pause { time: Some(time) }
    }
}

impl Default for Pause {
    fn default() -> Pause {
        Pause { time: None }
    }
}

impl Message for Pause {
    type Result = ();
}

/// Resume connection process for `ClientConnector`
#[derive(Message)]
pub struct Resume;

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Fail, Debug)]
pub enum ClientConnectorError {
    /// Invalid URL
    #[fail(display = "Invalid URL")]
    InvalidUrl,

    /// SSL feature is not enabled
    #[fail(display = "SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "alpn")]
    #[fail(display = "{}", _0)]
    SslError(#[cause] OpensslError),

    /// SSL error
    #[cfg(all(feature = "tls", not(feature = "alpn")))]
    #[fail(display = "{}", _0)]
    SslError(#[cause] TlsError),

    /// Resolver error
    #[fail(display = "{}", _0)]
    Resolver(#[cause] ResolverError),

    /// Connection took too long
    #[fail(display = "Timeout while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[fail(display = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Connection IO error
    #[fail(display = "{}", _0)]
    IoError(#[cause] io::Error),
}

impl From<ResolverError> for ClientConnectorError {
    fn from(err: ResolverError) -> ClientConnectorError {
        match err {
            ResolverError::Timeout => ClientConnectorError::Timeout,
            _ => ClientConnectorError::Resolver(err),
        }
    }
}

struct Waiter {
    tx: oneshot::Sender<Result<Connection, ClientConnectorError>>,
    wait: Instant,
    conn_timeout: Duration,
}

enum Paused {
    No,
    Yes,
    Timeout(Instant, Delay),
}

impl Paused {
    fn is_paused(&self) -> bool {
        match *self {
            Paused::No => false,
            _ => true,
        }
    }
}

/// `ClientConnector` type is responsible for transport layer of a
/// client connection.
pub struct ClientConnector {
    #[cfg(all(feature = "alpn"))]
    connector: SslConnector,
    #[cfg(all(feature = "tls", not(feature = "alpn")))]
    connector: TlsConnector,

    stats: ClientConnectorStats,
    subscriber: Option<Recipient<ClientConnectorStats>>,

    acq_tx: mpsc::UnboundedSender<AcquiredConnOperation>,
    acq_rx: Option<mpsc::UnboundedReceiver<AcquiredConnOperation>>,

    resolver: Option<Addr<Resolver>>,
    conn_lifetime: Duration,
    conn_keep_alive: Duration,
    limit: usize,
    limit_per_host: usize,
    acquired: usize,
    acquired_per_host: HashMap<Key, usize>,
    available: HashMap<Key, VecDeque<Conn>>,
    to_close: Vec<Connection>,
    waiters: Option<HashMap<Key, VecDeque<Waiter>>>,
    wait_timeout: Option<(Instant, Delay)>,
    paused: Paused,
}

impl Actor for ClientConnector {
    type Context = Context<ClientConnector>;

    fn started(&mut self, ctx: &mut Self::Context) {
        if self.resolver.is_none() {
            self.resolver = Some(Resolver::from_registry())
        }
        self.collect_periodic(ctx);
        ctx.add_stream(self.acq_rx.take().unwrap());
        ctx.spawn(Maintenance);
    }
}

impl Supervised for ClientConnector {}

impl SystemService for ClientConnector {}

impl Default for ClientConnector {
    fn default() -> ClientConnector {
        #[cfg(all(feature = "alpn"))]
        {
            let builder = SslConnector::builder(SslMethod::tls()).unwrap();
            ClientConnector::with_connector(builder.build())
        }
        #[cfg(all(feature = "tls", not(feature = "alpn")))]
        {
            let (tx, rx) = mpsc::unbounded();
            let builder = TlsConnector::builder().unwrap();
            ClientConnector {
                stats: ClientConnectorStats::default(),
                subscriber: None,
                acq_tx: tx,
                acq_rx: Some(rx),
                resolver: None,
                connector: builder.build().unwrap(),
                conn_lifetime: Duration::from_secs(75),
                conn_keep_alive: Duration::from_secs(15),
                limit: 100,
                limit_per_host: 0,
                acquired: 0,
                acquired_per_host: HashMap::new(),
                available: HashMap::new(),
                to_close: Vec::new(),
                waiters: Some(HashMap::new()),
                wait_timeout: None,
                paused: Paused::No,
            }
        }

        #[cfg(not(any(feature = "alpn", feature = "tls")))]
        {
            let (tx, rx) = mpsc::unbounded();
            ClientConnector {
                stats: ClientConnectorStats::default(),
                subscriber: None,
                acq_tx: tx,
                acq_rx: Some(rx),
                resolver: None,
                conn_lifetime: Duration::from_secs(75),
                conn_keep_alive: Duration::from_secs(15),
                limit: 100,
                limit_per_host: 0,
                acquired: 0,
                acquired_per_host: HashMap::new(),
                available: HashMap::new(),
                to_close: Vec::new(),
                waiters: Some(HashMap::new()),
                wait_timeout: None,
                paused: Paused::No,
            }
        }
    }
}

impl ClientConnector {
    #[cfg(feature = "alpn")]
    /// Create `ClientConnector` actor with custom `SslConnector` instance.
    ///
    /// By default `ClientConnector` uses very a simple SSL configuration.
    /// With `with_connector` method it is possible to use a custom
    /// `SslConnector` object.
    ///
    /// ```rust,ignore
    /// # #![cfg(feature="alpn")]
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # use futures::{future, Future};
    /// # use std::io::Write;
    /// # use std::process;
    /// # use actix_web::actix::Actor;
    /// extern crate openssl;
    /// use actix_web::{actix, client::ClientConnector, client::Connect};
    ///
    /// use openssl::ssl::{SslConnector, SslMethod};
    ///
    /// fn main() {
    ///     actix::run(|| {
    ///         // Start `ClientConnector` with custom `SslConnector`
    ///         let ssl_conn = SslConnector::builder(SslMethod::tls()).unwrap().build();
    ///         let conn = ClientConnector::with_connector(ssl_conn).start();
    ///
    ///         conn.send(
    ///             Connect::new("https://www.rust-lang.org").unwrap()) // <- connect to host
    ///                 .map_err(|_| ())
    ///                 .and_then(|res| {
    ///                     if let Ok(mut stream) = res {
    ///                         stream.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
    ///                     }
    /// #                   actix::System::current().stop();
    ///                     Ok(())
    ///                 })
    ///     );
    /// }
    /// ```
    pub fn with_connector(connector: SslConnector) -> ClientConnector {
        let (tx, rx) = mpsc::unbounded();

        ClientConnector {
            connector,
            stats: ClientConnectorStats::default(),
            subscriber: None,
            acq_tx: tx,
            acq_rx: Some(rx),
            resolver: None,
            conn_lifetime: Duration::from_secs(75),
            conn_keep_alive: Duration::from_secs(15),
            limit: 100,
            limit_per_host: 0,
            acquired: 0,
            acquired_per_host: HashMap::new(),
            available: HashMap::new(),
            to_close: Vec::new(),
            waiters: Some(HashMap::new()),
            wait_timeout: None,
            paused: Paused::No,
        }
    }

    /// Set total number of simultaneous connections.
    ///
    /// If limit is 0, the connector has no limit.
    /// The default limit size is 100.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set total number of simultaneous connections to the same endpoint.
    ///
    /// Endpoints are the same if they have equal (host, port, ssl) triplets.
    /// If limit is 0, the connector has no limit. The default limit size is 0.
    pub fn limit_per_host(mut self, limit: usize) -> Self {
        self.limit_per_host = limit;
        self
    }

    /// Set keep-alive period for opened connection.
    ///
    /// Keep-alive period is the period between connection usage. If
    /// the delay between repeated usages of the same connection
    /// exceeds this period, the connection is closed.
    /// Default keep-alive period is 15 seconds.
    pub fn conn_keep_alive(mut self, dur: Duration) -> Self {
        self.conn_keep_alive = dur;
        self
    }

    /// Set max lifetime period for connection.
    ///
    /// Connection lifetime is max lifetime of any opened connection
    /// until it is closed regardless of keep-alive period.
    /// Default lifetime period is 75 seconds.
    pub fn conn_lifetime(mut self, dur: Duration) -> Self {
        self.conn_lifetime = dur;
        self
    }

    /// Subscribe for connector stats. Only one subscriber is supported.
    pub fn stats(mut self, subs: Recipient<ClientConnectorStats>) -> Self {
        self.subscriber = Some(subs);
        self
    }

    /// Use custom resolver actor
    pub fn resolver(mut self, addr: Addr<Resolver>) -> Self {
        self.resolver = Some(addr);
        self
    }

    fn acquire(&mut self, key: &Key) -> Acquire {
        // check limits
        if self.limit > 0 {
            if self.acquired >= self.limit {
                return Acquire::NotAvailable;
            }
            if self.limit_per_host > 0 {
                if let Some(per_host) = self.acquired_per_host.get(key) {
                    if *per_host >= self.limit_per_host {
                        return Acquire::NotAvailable;
                    }
                }
            }
        } else if self.limit_per_host > 0 {
            if let Some(per_host) = self.acquired_per_host.get(key) {
                if *per_host >= self.limit_per_host {
                    return Acquire::NotAvailable;
                }
            }
        }

        self.reserve(key);

        // check if open connection is available
        // cleanup stale connections at the same time
        if let Some(ref mut connections) = self.available.get_mut(key) {
            let now = Instant::now();
            while let Some(conn) = connections.pop_back() {
                // check if it still usable
                if (now - conn.0) > self.conn_keep_alive
                    || (now - conn.1.ts) > self.conn_lifetime
                {
                    self.stats.closed += 1;
                    self.to_close.push(conn.1);
                } else {
                    let mut conn = conn.1;
                    let mut buf = [0; 2];
                    match conn.stream().read(&mut buf) {
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => (),
                        Ok(n) if n > 0 => {
                            self.stats.closed += 1;
                            self.to_close.push(conn);
                            continue;
                        }
                        Ok(_) | Err(_) => continue,
                    }
                    return Acquire::Acquired(conn);
                }
            }
        }
        Acquire::Available
    }

    fn reserve(&mut self, key: &Key) {
        self.acquired += 1;
        let per_host = if let Some(per_host) = self.acquired_per_host.get(key) {
            *per_host
        } else {
            0
        };
        self.acquired_per_host.insert(key.clone(), per_host + 1);
    }

    fn release_key(&mut self, key: &Key) {
        if self.acquired > 0 {
            self.acquired -= 1;
        }
        let per_host = if let Some(per_host) = self.acquired_per_host.get(key) {
            *per_host
        } else {
            return;
        };
        if per_host > 1 {
            self.acquired_per_host.insert(key.clone(), per_host - 1);
        } else {
            self.acquired_per_host.remove(key);
        }
    }

    fn collect_periodic(&mut self, ctx: &mut Context<Self>) {
        // check connections for shutdown
        let mut idx = 0;
        while idx < self.to_close.len() {
            match AsyncWrite::shutdown(&mut self.to_close[idx]) {
                Ok(Async::NotReady) => idx += 1,
                _ => {
                    self.to_close.swap_remove(idx);
                }
            }
        }

        // re-schedule next collect period
        ctx.run_later(Duration::from_secs(1), |act, ctx| act.collect_periodic(ctx));

        // send stats
        let mut stats = mem::replace(&mut self.stats, ClientConnectorStats::default());
        if let Some(ref mut subscr) = self.subscriber {
            if let Some(ref waiters) = self.waiters {
                for w in waiters.values() {
                    stats.wait_queue += w.len();
                }
            }
            let _ = subscr.do_send(stats);
        }
    }

    // TODO: waiters should be sorted by deadline. maybe timewheel?
    fn collect_waiters(&mut self) {
        let now = Instant::now();
        let mut next = None;

        for waiters in self.waiters.as_mut().unwrap().values_mut() {
            let mut idx = 0;
            while idx < waiters.len() {
                let wait = waiters[idx].wait;
                if wait <= now {
                    self.stats.timeouts += 1;
                    let waiter = waiters.swap_remove_back(idx).unwrap();
                    let _ = waiter.tx.send(Err(ClientConnectorError::Timeout));
                } else {
                    if let Some(n) = next {
                        if wait < n {
                            next = Some(wait);
                        }
                    } else {
                        next = Some(wait);
                    }
                    idx += 1;
                }
            }
        }

        if next.is_some() {
            self.install_wait_timeout(next.unwrap());
        }
    }

    fn install_wait_timeout(&mut self, time: Instant) {
        if let Some(ref mut wait) = self.wait_timeout {
            if wait.0 < time {
                return;
            }
        }

        let mut timeout = Delay::new(time);
        let _ = timeout.poll();
        self.wait_timeout = Some((time, timeout));
    }

    fn wait_for(
        &mut self, key: Key, wait: Duration, conn_timeout: Duration,
    ) -> oneshot::Receiver<Result<Connection, ClientConnectorError>> {
        // connection is not available, wait
        let (tx, rx) = oneshot::channel();

        let wait = Instant::now() + wait;
        self.install_wait_timeout(wait);

        let waiter = Waiter {
            tx,
            wait,
            conn_timeout,
        };
        self.waiters
            .as_mut()
            .unwrap()
            .entry(key)
            .or_insert_with(VecDeque::new)
            .push_back(waiter);
        rx
    }

    fn check_availibility(&mut self, ctx: &mut Context<ClientConnector>) {
        // check waiters
        let mut act_waiters = self.waiters.take().unwrap();

        for (key, ref mut waiters) in &mut act_waiters {
            while let Some(waiter) = waiters.pop_front() {
                if waiter.tx.is_canceled() {
                    continue;
                }

                match self.acquire(key) {
                    Acquire::Acquired(mut conn) => {
                        // use existing connection
                        self.stats.reused += 1;
                        conn.pool =
                            Some(AcquiredConn(key.clone(), Some(self.acq_tx.clone())));
                        let _ = waiter.tx.send(Ok(conn));
                    }
                    Acquire::NotAvailable => {
                        waiters.push_front(waiter);
                        break;
                    }
                    Acquire::Available => {
                        // create new connection
                        self.connect_waiter(&key, waiter, ctx);
                    }
                }
            }
        }

        self.waiters = Some(act_waiters);
    }

    fn connect_waiter(&mut self, key: &Key, waiter: Waiter, ctx: &mut Context<Self>) {
        let key = key.clone();
        let conn = AcquiredConn(key.clone(), Some(self.acq_tx.clone()));

        let key2 = key.clone();
        fut::WrapFuture::<ClientConnector>::actfuture(
            self.resolver.as_ref().unwrap().send(
                ResolveConnect::host_and_port(&conn.0.host, conn.0.port)
                    .timeout(waiter.conn_timeout),
            ),
        ).map_err(move |_, act, _| {
            act.release_key(&key2);
            ()
        })
            .and_then(move |res, act, _| {
                #[cfg(feature = "alpn")]
                match res {
                    Err(err) => {
                        let _ = waiter.tx.send(Err(err.into()));
                        fut::Either::B(fut::err(()))
                    }
                    Ok(stream) => {
                        act.stats.opened += 1;
                        if conn.0.ssl {
                            fut::Either::A(
                                act.connector
                                    .connect_async(&key.host, stream)
                                    .into_actor(act)
                                    .then(move |res, _, _| {
                                        match res {
                                            Err(e) => {
                                                let _ = waiter.tx.send(Err(
                                                    ClientConnectorError::SslError(e),
                                                ));
                                            }
                                            Ok(stream) => {
                                                let _ =
                                                    waiter.tx.send(Ok(Connection::new(
                                                        conn.0.clone(),
                                                        Some(conn),
                                                        Box::new(stream),
                                                    )));
                                            }
                                        }
                                        fut::ok(())
                                    }),
                            )
                        } else {
                            let _ = waiter.tx.send(Ok(Connection::new(
                                conn.0.clone(),
                                Some(conn),
                                Box::new(stream),
                            )));
                            fut::Either::B(fut::ok(()))
                        }
                    }
                }

                #[cfg(all(feature = "tls", not(feature = "alpn")))]
                match res {
                    Err(err) => {
                        let _ = waiter.tx.send(Err(err.into()));
                        fut::Either::B(fut::err(()))
                    }
                    Ok(stream) => {
                        act.stats.opened += 1;
                        if conn.0.ssl {
                            fut::Either::A(
                                act.connector
                                    .connect_async(&conn.0.host, stream)
                                    .into_actor(act)
                                    .then(move |res, _, _| {
                                        match res {
                                            Err(e) => {
                                                let _ = waiter.tx.send(Err(
                                                    ClientConnectorError::SslError(e),
                                                ));
                                            }
                                            Ok(stream) => {
                                                let _ =
                                                    waiter.tx.send(Ok(Connection::new(
                                                        conn.0.clone(),
                                                        Some(conn),
                                                        Box::new(stream),
                                                    )));
                                            }
                                        }
                                        fut::ok(())
                                    }),
                            )
                        } else {
                            let _ = waiter.tx.send(Ok(Connection::new(
                                conn.0.clone(),
                                Some(conn),
                                Box::new(stream),
                            )));
                            fut::Either::B(fut::ok(()))
                        }
                    }
                }

                #[cfg(not(any(feature = "alpn", feature = "tls")))]
                match res {
                    Err(err) => {
                        let _ = waiter.tx.send(Err(err.into()));
                        fut::err(())
                    }
                    Ok(stream) => {
                        act.stats.opened += 1;
                        if conn.0.ssl {
                            let _ = waiter
                                .tx
                                .send(Err(ClientConnectorError::SslIsNotSupported));
                        } else {
                            let _ = waiter.tx.send(Ok(Connection::new(
                                conn.0.clone(),
                                Some(conn),
                                Box::new(stream),
                            )));
                        };
                        fut::ok(())
                    }
                }
            })
            .spawn(ctx);
    }
}

impl Handler<Pause> for ClientConnector {
    type Result = ();

    fn handle(&mut self, msg: Pause, _: &mut Self::Context) {
        if let Some(time) = msg.time {
            let when = Instant::now() + time;
            let mut timeout = Delay::new(when);
            let _ = timeout.poll();
            self.paused = Paused::Timeout(when, timeout);
        } else {
            self.paused = Paused::Yes;
        }
    }
}

impl Handler<Resume> for ClientConnector {
    type Result = ();

    fn handle(&mut self, _: Resume, _: &mut Self::Context) {
        self.paused = Paused::No;
    }
}

impl Handler<Connect> for ClientConnector {
    type Result = ActorResponse<ClientConnector, Connection, ClientConnectorError>;

    fn handle(&mut self, msg: Connect, ctx: &mut Self::Context) -> Self::Result {
        let uri = &msg.uri;
        let wait_timeout = msg.wait_timeout;
        let conn_timeout = msg.conn_timeout;

        // host name is required
        if uri.host().is_none() {
            return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl));
        }

        // supported protocols
        let proto = match uri.scheme_part() {
            Some(scheme) => match Protocol::from(scheme.as_str()) {
                Some(proto) => proto,
                None => {
                    return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl))
                }
            },
            None => return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl)),
        };

        // check ssl availability
        if proto.is_secure() && !HAS_OPENSSL && !HAS_TLS {
            return ActorResponse::reply(Err(ClientConnectorError::SslIsNotSupported));
        }

        let host = uri.host().unwrap().to_owned();
        let port = uri.port().unwrap_or_else(|| proto.port());
        let key = Key {
            host,
            port,
            ssl: proto.is_secure(),
        };

        // check pause state
        if self.paused.is_paused() {
            let rx = self.wait_for(key.clone(), wait_timeout, conn_timeout);
            self.stats.waits += 1;
            return ActorResponse::async(
                rx.map_err(|_| ClientConnectorError::Disconnected)
                    .into_actor(self)
                    .and_then(move |res, act, ctx| match res {
                        Ok(conn) => fut::ok(conn),
                        Err(err) => {
                            match err {
                                ClientConnectorError::Timeout => (),
                                _ => {
                                    act.release_key(&key);
                                }
                            }
                            act.stats.errors += 1;
                            act.check_availibility(ctx);
                            fut::err(err)
                        }
                    }),
            );
        }

        // do not re-use websockets connection
        if !proto.is_http() {
            let (tx, rx) = oneshot::channel();
            let wait = Instant::now() + wait_timeout;
            let waiter = Waiter {
                tx,
                wait,
                conn_timeout,
            };
            self.connect_waiter(&key, waiter, ctx);

            return ActorResponse::async(
                rx.map_err(|_| ClientConnectorError::Disconnected)
                    .into_actor(self)
                    .and_then(move |res, act, ctx| match res {
                        Ok(conn) => fut::ok(conn),
                        Err(err) => {
                            act.stats.errors += 1;
                            act.release_key(&key);
                            act.check_availibility(ctx);
                            fut::err(err)
                        }
                    }),
            );
        }

        // acquire connection
        match self.acquire(&key) {
            Acquire::Acquired(mut conn) => {
                // use existing connection
                conn.pool = Some(AcquiredConn(key, Some(self.acq_tx.clone())));
                self.stats.reused += 1;
                ActorResponse::async(fut::ok(conn))
            }
            Acquire::NotAvailable => {
                // connection is not available, wait
                let rx = self.wait_for(key.clone(), wait_timeout, conn_timeout);
                self.stats.waits += 1;

                ActorResponse::async(
                    rx.map_err(|_| ClientConnectorError::Disconnected)
                        .into_actor(self)
                        .and_then(move |res, act, ctx| match res {
                            Ok(conn) => fut::ok(conn),
                            Err(err) => {
                                match err {
                                    ClientConnectorError::Timeout => (),
                                    _ => {
                                        act.release_key(&key);
                                    }
                                }
                                act.stats.errors += 1;
                                act.check_availibility(ctx);
                                fut::err(err)
                            }
                        }),
                )
            }
            Acquire::Available => {
                let (tx, rx) = oneshot::channel();
                let wait = Instant::now() + wait_timeout;
                let waiter = Waiter {
                    tx,
                    wait,
                    conn_timeout,
                };
                self.connect_waiter(&key, waiter, ctx);

                ActorResponse::async(
                    rx.map_err(|_| ClientConnectorError::Disconnected)
                        .into_actor(self)
                        .and_then(move |res, act, ctx| match res {
                            Ok(conn) => fut::ok(conn),
                            Err(err) => {
                                act.stats.errors += 1;
                                act.release_key(&key);
                                act.check_availibility(ctx);
                                fut::err(err)
                            }
                        }),
                )
            }
        }
    }
}

impl StreamHandler<AcquiredConnOperation, ()> for ClientConnector {
    fn handle(&mut self, msg: AcquiredConnOperation, ctx: &mut Context<Self>) {
        match msg {
            AcquiredConnOperation::Close(conn) => {
                self.release_key(&conn.key);
                self.to_close.push(conn);
                self.stats.closed += 1;
            }
            AcquiredConnOperation::Release(conn) => {
                self.release_key(&conn.key);
                if (Instant::now() - conn.ts) < self.conn_lifetime {
                    self.available
                        .entry(conn.key.clone())
                        .or_insert_with(VecDeque::new)
                        .push_back(Conn(Instant::now(), conn));
                } else {
                    self.to_close.push(conn);
                    self.stats.closed += 1;
                }
            }
            AcquiredConnOperation::ReleaseKey(key) => {
                // closed
                self.stats.closed += 1;
                self.release_key(&key);
            }
        }

        self.check_availibility(ctx);
    }
}

struct Maintenance;

impl fut::ActorFuture for Maintenance {
    type Item = ();
    type Error = ();
    type Actor = ClientConnector;

    fn poll(
        &mut self, act: &mut ClientConnector, ctx: &mut Context<ClientConnector>,
    ) -> Poll<Self::Item, Self::Error> {
        // check pause duration
        if let Paused::Timeout(inst, _) = act.paused {
            if inst <= Instant::now() {
                act.paused = Paused::No;
            }
        }

        // collect wait timers
        act.collect_waiters();

        // check waiters
        act.check_availibility(ctx);

        Ok(Async::NotReady)
    }
}

#[derive(PartialEq, Hash, Debug, Clone, Copy)]
enum Protocol {
    Http,
    Https,
    Ws,
    Wss,
}

impl Protocol {
    fn from(s: &str) -> Option<Protocol> {
        match s {
            "http" => Some(Protocol::Http),
            "https" => Some(Protocol::Https),
            "ws" => Some(Protocol::Ws),
            "wss" => Some(Protocol::Wss),
            _ => None,
        }
    }

    fn is_http(self) -> bool {
        match self {
            Protocol::Https | Protocol::Http => true,
            _ => false,
        }
    }

    fn is_secure(self) -> bool {
        match self {
            Protocol::Https | Protocol::Wss => true,
            _ => false,
        }
    }

    fn port(self) -> u16 {
        match self {
            Protocol::Http | Protocol::Ws => 80,
            Protocol::Https | Protocol::Wss => 443,
        }
    }
}

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct Key {
    host: String,
    port: u16,
    ssl: bool,
}

impl Key {
    fn empty() -> Key {
        Key {
            host: String::new(),
            port: 0,
            ssl: false,
        }
    }
}

#[derive(Debug)]
struct Conn(Instant, Connection);

enum Acquire {
    Acquired(Connection),
    Available,
    NotAvailable,
}

enum AcquiredConnOperation {
    Close(Connection),
    Release(Connection),
    ReleaseKey(Key),
}

struct AcquiredConn(Key, Option<mpsc::UnboundedSender<AcquiredConnOperation>>);

impl AcquiredConn {
    fn close(&mut self, conn: Connection) {
        if let Some(tx) = self.1.take() {
            let _ = tx.unbounded_send(AcquiredConnOperation::Close(conn));
        }
    }
    fn release(&mut self, conn: Connection) {
        if let Some(tx) = self.1.take() {
            let _ = tx.unbounded_send(AcquiredConnOperation::Release(conn));
        }
    }
}

impl Drop for AcquiredConn {
    fn drop(&mut self) {
        if let Some(tx) = self.1.take() {
            let _ = tx.unbounded_send(AcquiredConnOperation::ReleaseKey(self.0.clone()));
        }
    }
}

/// HTTP client connection
pub struct Connection {
    key: Key,
    stream: Box<IoStream + Send>,
    pool: Option<AcquiredConn>,
    ts: Instant,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Connection {}:{}", self.key.host, self.key.port)
    }
}

impl Connection {
    fn new(key: Key, pool: Option<AcquiredConn>, stream: Box<IoStream + Send>) -> Self {
        Connection {
            key,
            stream,
            pool,
            ts: Instant::now(),
        }
    }

    /// Raw IO stream
    pub fn stream(&mut self) -> &mut IoStream {
        &mut *self.stream
    }

    /// Create a new connection from an IO Stream
    pub fn from_stream<T: IoStream + Send>(io: T) -> Connection {
        Connection::new(Key::empty(), None, Box::new(io))
    }

    /// Close connection
    pub fn close(mut self) {
        if let Some(mut pool) = self.pool.take() {
            pool.close(self)
        }
    }

    /// Release this connection to the connection pool
    pub fn release(mut self) {
        if let Some(mut pool) = self.pool.take() {
            pool.release(self)
        }
    }
}

impl IoStream for Connection {
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        IoStream::shutdown(&mut *self.stream, how)
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        IoStream::set_nodelay(&mut *self.stream, nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        IoStream::set_linger(&mut *self.stream, dur)
    }
}

impl io::Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl AsyncRead for Connection {}

impl io::Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl AsyncWrite for Connection {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.stream.shutdown()
    }
}

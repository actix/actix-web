use std::{fmt, io, time};
use std::cell::RefCell;
use std::rc::Rc;
use std::net::Shutdown;
use std::time::{Duration, Instant};
use std::collections::{HashMap, VecDeque};

use actix::{fut, Actor, ActorFuture, Context, AsyncContext,
            Handler, Message, ActorResponse, Supervised};
use actix::registry::ArbiterService;
use actix::fut::WrapFuture;
use actix::actors::{Connector, ConnectorError, Connect as ResolveConnect};

use http::{Uri, HttpTryFrom, Error as HttpError};
use futures::{Async, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

#[cfg(feature="alpn")]
use openssl::ssl::{SslMethod, SslConnector, Error as OpensslError};
#[cfg(feature="alpn")]
use tokio_openssl::SslConnectorExt;
#[cfg(feature="alpn")]
use futures::Future;

#[cfg(all(feature="tls", not(feature="alpn")))]
use native_tls::{TlsConnector, Error as TlsError};
#[cfg(all(feature="tls", not(feature="alpn")))]
use tokio_tls::TlsConnectorExt;
#[cfg(all(feature="tls", not(feature="alpn")))]
use futures::Future;

use {HAS_OPENSSL, HAS_TLS};
use server::IoStream;


#[derive(Debug)]
/// `Connect` type represents message that can be send to `ClientConnector`
/// with connection request.
pub struct Connect {
    pub uri: Uri,
    pub conn_timeout: Duration,
}

impl Connect {
    /// Create `Connect` message for specified `Uri`
    pub fn new<U>(uri: U) -> Result<Connect, HttpError> where Uri: HttpTryFrom<U> {
        Ok(Connect {
            uri: Uri::try_from(uri).map_err(|e| e.into())?,
            conn_timeout: Duration::from_secs(1)
        })
    }
}

impl Message for Connect {
    type Result = Result<Connection, ClientConnectorError>;
}

/// A set of errors that can occur during connecting to a http host
#[derive(Fail, Debug)]
pub enum ClientConnectorError {
    /// Invalid url
    #[fail(display="Invalid url")]
    InvalidUrl,

    /// SSL feature is not enabled
    #[fail(display="SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature="alpn")]
    #[fail(display="{}", _0)]
    SslError(#[cause] OpensslError),

    /// SSL error
    #[cfg(all(feature="tls", not(feature="alpn")))]
    #[fail(display="{}", _0)]
    SslError(#[cause] TlsError),

    /// Connection error
    #[fail(display = "{}", _0)]
    Connector(#[cause] ConnectorError),

    /// Connection took too long
    #[fail(display = "Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[fail(display = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Connection io error
    #[fail(display = "{}", _0)]
    IoError(#[cause] io::Error),
}

impl From<ConnectorError> for ClientConnectorError {
    fn from(err: ConnectorError) -> ClientConnectorError {
        match err {
            ConnectorError::Timeout => ClientConnectorError::Timeout,
            _ => ClientConnectorError::Connector(err)
        }
    }
}

pub struct ClientConnector {
    #[cfg(all(feature="alpn"))]
    connector: SslConnector,
    #[cfg(all(feature="tls", not(feature="alpn")))]
    connector: TlsConnector,
    pool: Rc<Pool>,
}

impl Actor for ClientConnector {
    type Context = Context<ClientConnector>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.collect(ctx);
    }
}

impl Supervised for ClientConnector {}

impl ArbiterService for ClientConnector {}

impl Default for ClientConnector {
    fn default() -> ClientConnector {
        #[cfg(all(feature="alpn"))]
        {
            let builder = SslConnector::builder(SslMethod::tls()).unwrap();
            ClientConnector {
                connector: builder.build(),
                pool: Rc::new(Pool::new()),
            }
        }
        #[cfg(all(feature="tls", not(feature="alpn")))]
        {
            let builder = TlsConnector::builder().unwrap();
            ClientConnector {
                connector: builder.build().unwrap(),
                pool: Rc::new(Pool::new()),
            }
        }

        #[cfg(not(any(feature="alpn", feature="tls")))]
        ClientConnector {pool: Rc::new(Pool::new())}
    }
}

impl ClientConnector {

    #[cfg(feature="alpn")]
    /// Create `ClientConnector` actor with custom `SslConnector` instance.
    ///
    /// By default `ClientConnector` uses very simple ssl configuration.
    /// With `with_connector` method it is possible to use custom `SslConnector`
    /// object.
    ///
    /// ```rust
    /// # #![cfg(feature="alpn")]
    /// # extern crate actix;
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # use futures::Future;
    /// # use std::io::Write;
    /// extern crate openssl;
    /// use actix::prelude::*;
    /// use actix_web::client::{Connect, ClientConnector};
    ///
    /// use openssl::ssl::{SslMethod, SslConnector};
    ///
    /// fn main() {
    ///     let sys = System::new("test");
    ///
    ///     // Start `ClientConnector` with custom `SslConnector`
    ///     let ssl_conn = SslConnector::builder(SslMethod::tls()).unwrap().build();
    ///     let conn: Address<_> = ClientConnector::with_connector(ssl_conn).start();
    ///
    ///     Arbiter::handle().spawn({
    ///         conn.send(
    ///             Connect::new("https://www.rust-lang.org").unwrap()) // <- connect to host
    ///                 .map_err(|_| ())
    ///                 .and_then(|res| {
    ///                     if let Ok(mut stream) = res {
    ///                         stream.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
    ///                     }
    /// #                   Arbiter::system().do_send(actix::msgs::SystemExit(0));
    ///                     Ok(())
    ///                 })
    ///     });
    ///
    ///     sys.run();
    /// }
    /// ```
    pub fn with_connector(connector: SslConnector) -> ClientConnector {
        ClientConnector { connector, pool: Rc::new(Pool::new()) }
    }

    fn collect(&mut self, ctx: &mut Context<Self>) {
        self.pool.collect();
        ctx.run_later(Duration::from_secs(1), |act, ctx| act.collect(ctx));
    }
}

impl Handler<Connect> for ClientConnector {
    type Result = ActorResponse<ClientConnector, Connection, ClientConnectorError>;

    fn handle(&mut self, msg: Connect, _: &mut Self::Context) -> Self::Result {
        let uri = &msg.uri;
        let conn_timeout = msg.conn_timeout;

        // host name is required
        if uri.host().is_none() {
            return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl))
        }

        // supported protocols
        let proto = match uri.scheme_part() {
            Some(scheme) => match Protocol::from(scheme.as_str()) {
                Some(proto) => proto,
                None => return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl)),
            },
            None => return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl)),
        };

        // check ssl availability
        if proto.is_secure() && !HAS_OPENSSL && !HAS_TLS {
            return ActorResponse::reply(Err(ClientConnectorError::SslIsNotSupported))
        }

        let host = uri.host().unwrap().to_owned();
        let port = uri.port().unwrap_or_else(|| proto.port());
        let key = Key {host, port, ssl: proto.is_secure()};

        let pool = if proto.is_http() {
            if let Some(conn) = self.pool.query(&key) {
                return ActorResponse::async(fut::ok(conn))
            } else {
                Some(Rc::clone(&self.pool))
            }
        } else {
            None
        };

        ActorResponse::async(
            Connector::from_registry()
                .send(ResolveConnect::host_and_port(&key.host, port)
                      .timeout(conn_timeout))
                .into_actor(self)
                .map_err(|_, _, _| ClientConnectorError::Disconnected)
                .and_then(move |res, _act, _| {
                    #[cfg(feature="alpn")]
                    match res {
                        Err(err) => fut::Either::B(fut::err(err.into())),
                        Ok(stream) => {
                            if proto.is_secure() {
                                fut::Either::A(
                                    _act.connector.connect_async(&key.host, stream)
                                        .map_err(ClientConnectorError::SslError)
                                        .map(|stream| Connection::new(
                                            key, pool, Box::new(stream)))
                                        .into_actor(_act))
                            } else {
                                fut::Either::B(fut::ok(
                                    Connection::new(key, pool, Box::new(stream))))
                            }
                        }
                    }

                    #[cfg(all(feature="tls", not(feature="alpn")))]
                    match res {
                        Err(err) => fut::Either::B(fut::err(err.into())),
                        Ok(stream) => {
                            if proto.is_secure() {
                                fut::Either::A(
                                    _act.connector.connect_async(&key.host, stream)
                                        .map_err(ClientConnectorError::SslError)
                                        .map(|stream| Connection::new(
                                            key, pool, Box::new(stream)))
                                        .into_actor(_act))
                            } else {
                                fut::Either::B(fut::ok(
                                    Connection::new(key, pool, Box::new(stream))))
                            }
                        }
                    }

                    #[cfg(not(any(feature="alpn", feature="tls")))]
                    match res {
                        Err(err) => fut::err(err.into()),
                        Ok(stream) => {
                            if proto.is_secure() {
                                fut::err(ClientConnectorError::SslIsNotSupported)
                            } else {
                                fut::ok(Connection::new(key, pool, Box::new(stream)))
                            }
                        }
                    }
                }))
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

    fn is_http(&self) -> bool {
        match *self {
            Protocol::Https | Protocol::Http => true,
            _ => false,
        }
    }

    fn is_secure(&self) -> bool {
        match *self {
            Protocol::Https | Protocol::Wss => true,
            _ => false,
        }
    }

    fn port(&self) -> u16 {
        match *self {
            Protocol::Http | Protocol::Ws => 80,
            Protocol::Https | Protocol::Wss => 443
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
        Key{host: String::new(), port: 0, ssl: false}
    }
}

#[derive(Debug)]
struct Conn(Instant, Connection);

pub struct Pool {
    max_size: usize,
    keep_alive: Duration,
    max_lifetime: Duration,
    pool: RefCell<HashMap<Key, VecDeque<Conn>>>,
    to_close: RefCell<Vec<Connection>>,
}

impl Pool {
    fn new() -> Pool {
        Pool {
            max_size: 128,
            keep_alive: Duration::from_secs(15),
            max_lifetime: Duration::from_secs(75),
            pool: RefCell::new(HashMap::new()),
            to_close: RefCell::new(Vec::new()),
        }
    }

    fn collect(&self) {
        let mut pool = self.pool.borrow_mut();
        let mut to_close = self.to_close.borrow_mut();

        // check keep-alive
        let now = Instant::now();
        for conns in pool.values_mut() {
            while !conns.is_empty() {
                if (now - conns[0].0) > self.keep_alive
                    || (now - conns[0].1.ts) > self.max_lifetime
                {
                    let conn = conns.pop_front().unwrap().1;
                    to_close.push(conn);
                } else {
                    break
                }
            }
        }

        // check connections for shutdown
        let mut idx = 0;
        while idx < to_close.len() {
            match AsyncWrite::shutdown(&mut to_close[idx]) {
                Ok(Async::NotReady) => idx += 1,
                _ => {
                    to_close.swap_remove(idx);
                },
            }
        }
    }

    fn query(&self, key: &Key) -> Option<Connection> {
        let mut pool = self.pool.borrow_mut();
        let mut to_close = self.to_close.borrow_mut();

        if let Some(ref mut connections) = pool.get_mut(key) {
            let now = Instant::now();
            while let Some(conn) = connections.pop_back() {
                // check if it still usable
                if (now - conn.0) > self.keep_alive
                    || (now - conn.1.ts) > self.max_lifetime
                {
                    to_close.push(conn.1);
                } else {
                    let mut conn = conn.1;
                    let mut buf = [0; 2];
                    match conn.stream().read(&mut buf) {
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => (),
                        Ok(n) if n > 0 => {
                            to_close.push(conn);
                            continue
                        },
                        Ok(_) | Err(_) => continue,
                    }
                    return Some(conn)
                }
            }
        }
        None
    }

    fn release(&self, conn: Connection) {
        if (Instant::now() - conn.ts) < self.max_lifetime {
            let mut pool = self.pool.borrow_mut();
            if !pool.contains_key(&conn.key) {
                let key = conn.key.clone();
                let mut vec = VecDeque::new();
                vec.push_back(Conn(Instant::now(), conn));
                pool.insert(key, vec);
            } else {
                let vec = pool.get_mut(&conn.key).unwrap();
                vec.push_back(Conn(Instant::now(), conn));
                if vec.len() > self.max_size {
                    let conn = vec.pop_front().unwrap();
                    self.to_close.borrow_mut().push(conn.1);
                }
            }
        }
    }
}


pub struct Connection {
    key: Key,
    stream: Box<IoStream>,
    pool: Option<Rc<Pool>>,
    ts: Instant,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Connection {}:{}", self.key.host, self.key.port)
    }
}

impl Connection {
    fn new(key: Key, pool: Option<Rc<Pool>>, stream: Box<IoStream>) -> Self {
        Connection {
            key, pool, stream,
            ts: Instant::now(),
        }
    }

    pub fn stream(&mut self) -> &mut IoStream {
        &mut *self.stream
    }

    pub fn from_stream<T: IoStream>(io: T) -> Connection {
        Connection::new(Key::empty(), None, Box::new(io))
    }

    pub fn release(mut self) {
        if let Some(pool) = self.pool.take() {
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

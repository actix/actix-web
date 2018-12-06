use std::collections::VecDeque;
use std::marker::PhantomData;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use std::{fmt, io};

use futures::future::{ok, Either, FutureResult};
use futures::{try_ready, Async, Future, Poll};

use tokio_tcp::{ConnectFuture, TcpStream};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::system_conf::read_system_conf;

use super::resolver::{RequestHost, ResolveError, Resolver, ResolverFuture};
use super::service::{NewService, Service};

/// Port of the request
pub trait RequestPort {
    fn port(&self) -> u16;
}

// #[derive(Fail, Debug)]
#[derive(Debug)]
pub enum ConnectorError {
    /// Failed to resolve the hostname
    // #[fail(display = "Failed resolving hostname: {}", _0)]
    Resolver(ResolveError),

    /// No dns records
    // #[fail(display = "No dns records found for the input")]
    NoRecords,

    /// Connecting took too long
    // #[fail(display = "Timeout out while establishing connection")]
    Timeout,

    /// Invalid input
    InvalidInput,

    /// Connection io error
    // #[fail(display = "{}", _0)]
    IoError(io::Error),
}

impl From<ResolveError> for ConnectorError {
    fn from(err: ResolveError) -> Self {
        ConnectorError::Resolver(err)
    }
}

impl From<io::Error> for ConnectorError {
    fn from(err: io::Error) -> Self {
        ConnectorError::IoError(err)
    }
}

/// Connect request
#[derive(Eq, PartialEq, Debug, Hash)]
pub struct Connect {
    pub kind: ConnectKind,
    pub timeout: Duration,
}

#[derive(Eq, PartialEq, Debug, Hash)]
pub enum ConnectKind {
    Host { host: String, port: u16 },
    Addr { host: String, addr: SocketAddr },
}

impl Connect {
    /// Create new `Connect` instance.
    pub fn new<T: AsRef<str>>(host: T, port: u16) -> Connect {
        Connect {
            kind: ConnectKind::Host {
                host: host.as_ref().to_owned(),
                port,
            },
            timeout: Duration::from_secs(1),
        }
    }

    /// Create `Connect` instance by spliting the string by ':' and convert the second part to u16
    pub fn with<T: AsRef<str>>(host: T) -> Result<Connect, ConnectorError> {
        let mut parts_iter = host.as_ref().splitn(2, ':');
        let host = parts_iter.next().ok_or(ConnectorError::InvalidInput)?;
        let port_str = parts_iter.next().unwrap_or("");
        let port = port_str
            .parse::<u16>()
            .map_err(|_| ConnectorError::InvalidInput)?;
        Ok(Connect {
            kind: ConnectKind::Host {
                host: host.to_owned(),
                port,
            },
            timeout: Duration::from_secs(1),
        })
    }

    /// Create new `Connect` instance from host and address. Connector skips name resolution stage for such connect messages.
    pub fn with_address<T: Into<String>>(host: T, addr: SocketAddr) -> Connect {
        Connect {
            kind: ConnectKind::Addr {
                addr,
                host: host.into(),
            },
            timeout: Duration::from_secs(1),
        }
    }

    /// Set connect timeout
    ///
    /// By default timeout is set to a 1 second.
    pub fn timeout(mut self, timeout: Duration) -> Connect {
        self.timeout = timeout;
        self
    }
}

impl RequestHost for Connect {
    fn host(&self) -> &str {
        match self.kind {
            ConnectKind::Host { ref host, .. } => host,
            ConnectKind::Addr { ref host, .. } => host,
        }
    }
}

impl RequestPort for Connect {
    fn port(&self) -> u16 {
        match self.kind {
            ConnectKind::Host { port, .. } => port,
            ConnectKind::Addr { addr, .. } => addr.port(),
        }
    }
}

impl fmt::Display for Connect {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.host(), self.port())
    }
}

/// Tcp connector
pub struct Connector {
    resolver: Resolver<Connect>,
}

impl Default for Connector {
    fn default() -> Self {
        let (cfg, opts) = if let Ok((cfg, opts)) = read_system_conf() {
            (cfg, opts)
        } else {
            (ResolverConfig::default(), ResolverOpts::default())
        };

        Connector::new(cfg, opts)
    }
}

impl Connector {
    /// Create new connector with resolver configuration
    pub fn new(cfg: ResolverConfig, opts: ResolverOpts) -> Self {
        Connector {
            resolver: Resolver::new(cfg, opts),
        }
    }

    /// Create new connector with custom resolver
    pub fn with_resolver(
        resolver: Resolver<Connect>,
    ) -> impl Service<Connect, Response = (Connect, TcpStream), Error = ConnectorError> + Clone
    {
        Connector { resolver }
    }

    /// Create new default connector service
    pub fn new_service_with_config<E>(
        cfg: ResolverConfig,
        opts: ResolverOpts,
    ) -> impl NewService<
        Connect,
        Response = (Connect, TcpStream),
        Error = ConnectorError,
        InitError = E,
    > + Clone {
        move || -> FutureResult<Connector, E> { ok(Connector::new(cfg.clone(), opts)) }
    }
}

impl Clone for Connector {
    fn clone(&self) -> Self {
        Connector {
            resolver: self.resolver.clone(),
        }
    }
}

impl Service<Connect> for Connector {
    type Response = (Connect, TcpStream);
    type Error = ConnectorError;
    type Future = Either<ConnectorFuture, ConnectorTcpFuture>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Connect) -> Self::Future {
        match req.kind {
            ConnectKind::Host { .. } => Either::A(ConnectorFuture {
                fut: self.resolver.call(req),
                fut2: None,
            }),
            ConnectKind::Addr { addr, .. } => {
                let mut addrs = VecDeque::new();
                addrs.push_back(addr.ip());
                Either::B(ConnectorTcpFuture {
                    fut: TcpConnectorResponse::new(req, addrs),
                })
            }
        }
    }
}

#[doc(hidden)]
pub struct ConnectorFuture {
    fut: ResolverFuture<Connect>,
    fut2: Option<TcpConnectorResponse<Connect>>,
}

impl Future for ConnectorFuture {
    type Item = (Connect, TcpStream);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return fut.poll().map_err(ConnectorError::from);
        }
        match self.fut.poll().map_err(ConnectorError::from)? {
            Async::Ready((req, addrs)) => {
                if addrs.is_empty() {
                    Err(ConnectorError::NoRecords)
                } else {
                    self.fut2 = Some(TcpConnectorResponse::new(req, addrs));
                    self.poll()
                }
            }
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

#[doc(hidden)]
pub struct ConnectorTcpFuture {
    fut: TcpConnectorResponse<Connect>,
}

impl Future for ConnectorTcpFuture {
    type Item = (Connect, TcpStream);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.fut.poll().map_err(ConnectorError::IoError)
    }
}

/// Tcp stream connector service
pub struct TcpConnector<T: RequestPort>(PhantomData<T>);

impl<T: RequestPort> Default for TcpConnector<T> {
    fn default() -> TcpConnector<T> {
        TcpConnector(PhantomData)
    }
}

impl<T: RequestPort> Service<(T, VecDeque<IpAddr>)> for TcpConnector<T> {
    type Response = (T, TcpStream);
    type Error = io::Error;
    type Future = TcpConnectorResponse<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, addrs): (T, VecDeque<IpAddr>)) -> Self::Future {
        TcpConnectorResponse::new(req, addrs)
    }
}

#[doc(hidden)]
/// Tcp stream connector response future
pub struct TcpConnectorResponse<T: RequestPort> {
    port: u16,
    req: Option<T>,
    addr: Option<SocketAddr>,
    addrs: VecDeque<IpAddr>,
    stream: Option<ConnectFuture>,
}

impl<T: RequestPort> TcpConnectorResponse<T> {
    pub fn new(req: T, addrs: VecDeque<IpAddr>) -> TcpConnectorResponse<T> {
        TcpConnectorResponse {
            addrs,
            port: req.port(),
            req: Some(req),
            addr: None,
            stream: None,
        }
    }
}

impl<T: RequestPort> Future for TcpConnectorResponse<T> {
    type Item = (T, TcpStream);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // connect
        loop {
            if let Some(new) = self.stream.as_mut() {
                match new.poll() {
                    Ok(Async::Ready(sock)) => {
                        return Ok(Async::Ready((self.req.take().unwrap(), sock)))
                    }
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(err) => {
                        if self.addrs.is_empty() {
                            return Err(err);
                        }
                    }
                }
            }

            // try to connect
            let addr = SocketAddr::new(self.addrs.pop_front().unwrap(), self.port);
            self.stream = Some(TcpStream::connect(&addr));
            self.addr = Some(addr)
        }
    }
}

#[derive(Clone)]
pub struct DefaultConnector(Connector);

impl Default for DefaultConnector {
    fn default() -> Self {
        DefaultConnector(Connector::default())
    }
}

impl DefaultConnector {
    pub fn new(cfg: ResolverConfig, opts: ResolverOpts) -> Self {
        DefaultConnector(Connector::new(cfg, opts))
    }
}

impl Service<Connect> for DefaultConnector {
    type Response = TcpStream;
    type Error = ConnectorError;
    type Future = DefaultConnectorFuture;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.0.poll_ready()
    }

    fn call(&mut self, req: Connect) -> Self::Future {
        DefaultConnectorFuture {
            fut: self.0.call(req),
        }
    }
}

#[doc(hidden)]
pub struct DefaultConnectorFuture {
    fut: Either<ConnectorFuture, ConnectorTcpFuture>,
}

impl Future for DefaultConnectorFuture {
    type Item = TcpStream;
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(try_ready!(self.fut.poll()).1))
    }
}

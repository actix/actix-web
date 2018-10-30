use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;
use std::{fmt, io};

use futures::{
    future::{ok, FutureResult},
    Async, Future, Poll,
};
use tokio_tcp::{ConnectFuture, TcpStream};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::system_conf::read_system_conf;

use super::resolver::{HostAware, ResolveError, Resolver, ResolverFuture};
use super::service::{NewService, Service};

// #[derive(Fail, Debug)]
#[derive(Debug)]
pub enum ConnectorError {
    /// Failed to resolve the hostname
    // #[fail(display = "Failed resolving hostname: {}", _0)]
    Resolver(ResolveError),

    /// No dns records
    // #[fail(display = "Invalid input: {}", _0)]
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

/// Connect request
#[derive(Eq, PartialEq, Debug, Hash)]
pub struct Connect {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
}

impl Connect {
    /// Create new `Connect` instance.
    pub fn new<T: AsRef<str>>(host: T, port: u16) -> Connect {
        Connect {
            port,
            host: host.as_ref().to_owned(),
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
            port,
            host: host.to_owned(),
            timeout: Duration::from_secs(1),
        })
    }

    /// Set connect timeout
    ///
    /// By default timeout is set to a 1 second.
    pub fn timeout(mut self, timeout: Duration) -> Connect {
        self.timeout = timeout;
        self
    }
}

impl HostAware for Connect {
    fn host(&self) -> &str {
        &self.host
    }
}

impl fmt::Display for Connect {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
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
    ) -> impl Service<Request = Connect, Response = (Connect, TcpStream), Error = ConnectorError>
                 + Clone {
        Connector { resolver }
    }

    /// Create new default connector service
    pub fn new_service_with_config<E>(
        cfg: ResolverConfig,
        opts: ResolverOpts,
    ) -> impl NewService<
        Request = Connect,
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

impl Service for Connector {
    type Request = Connect;
    type Response = (Connect, TcpStream);
    type Error = ConnectorError;
    type Future = ConnectorFuture;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        ConnectorFuture {
            fut: self.resolver.call(req),
            fut2: None,
        }
    }
}

#[doc(hidden)]
pub struct ConnectorFuture {
    fut: ResolverFuture<Connect>,
    fut2: Option<TcpConnector>,
}

impl Future for ConnectorFuture {
    type Item = (Connect, TcpStream);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return fut.poll();
        }
        match self.fut.poll().map_err(ConnectorError::from)? {
            Async::Ready((req, mut addrs)) => {
                if addrs.is_empty() {
                    Err(ConnectorError::NoRecords)
                } else {
                    for addr in &mut addrs {
                        match addr {
                            SocketAddr::V4(ref mut addr) => addr.set_port(req.port),
                            SocketAddr::V6(ref mut addr) => addr.set_port(req.port),
                        }
                    }
                    self.fut2 = Some(TcpConnector::new(req, addrs));
                    self.poll()
                }
            }
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

#[doc(hidden)]
/// Tcp stream connector
pub struct TcpConnector {
    req: Option<Connect>,
    addr: Option<SocketAddr>,
    addrs: VecDeque<SocketAddr>,
    stream: Option<ConnectFuture>,
}

impl TcpConnector {
    pub fn new(req: Connect, addrs: VecDeque<SocketAddr>) -> TcpConnector {
        TcpConnector {
            addrs,
            req: Some(req),
            addr: None,
            stream: None,
        }
    }
}

impl Future for TcpConnector {
    type Item = (Connect, TcpStream);
    type Error = ConnectorError;

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
                            return Err(ConnectorError::IoError(err));
                        }
                    }
                }
            }

            // try to connect
            let addr = self.addrs.pop_front().unwrap();
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

impl Service for DefaultConnector {
    type Request = Connect;
    type Response = TcpStream;
    type Error = ConnectorError;
    type Future = DefaultConnectorFuture;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.0.poll_ready()
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        DefaultConnectorFuture {
            fut: self.0.call(req),
        }
    }
}

#[doc(hidden)]
pub struct DefaultConnectorFuture {
    fut: ConnectorFuture,
}

impl Future for DefaultConnectorFuture {
    type Item = TcpStream;
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(try_ready!(self.fut.poll()).1))
    }
}

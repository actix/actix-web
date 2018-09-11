use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;

use futures::{
    future::{ok, FutureResult},
    Async, Future, Poll,
};
use tokio_tcp::{ConnectFuture, TcpStream};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::system_conf::read_system_conf;

use super::resolver::{HostAware, Resolver, ResolverError, ResolverFuture};
use super::service::{NewService, Service};

// #[derive(Fail, Debug)]
#[derive(Debug)]
pub enum ConnectorError {
    /// Failed to resolve the hostname
    // #[fail(display = "Failed resolving hostname: {}", _0)]
    Resolver(ResolverError),

    /// Not dns records
    // #[fail(display = "Invalid input: {}", _0)]
    NoRecords,

    /// Connection io error
    // #[fail(display = "{}", _0)]
    IoError(io::Error),
}

impl From<ResolverError> for ConnectorError {
    fn from(err: ResolverError) -> Self {
        ConnectorError::Resolver(err)
    }
}

pub struct ConnectionInfo {
    pub host: String,
    pub addr: SocketAddr,
}

pub struct Connector<T = String> {
    resolver: Resolver<T>,
}

impl<T: HostAware> Default for Connector<T> {
    fn default() -> Self {
        let (cfg, opts) = if let Ok((cfg, opts)) = read_system_conf() {
            (cfg, opts)
        } else {
            (ResolverConfig::default(), ResolverOpts::default())
        };

        Connector::new(cfg, opts)
    }
}

impl<T: HostAware> Connector<T> {
    pub fn new(cfg: ResolverConfig, opts: ResolverOpts) -> Self {
        Connector {
            resolver: Resolver::new(cfg, opts),
        }
    }

    pub fn with_resolver(
        resolver: Resolver<T>,
    ) -> impl Service<
        Request = T,
        Response = (T, ConnectionInfo, TcpStream),
        Error = ConnectorError,
    > + Clone {
        Connector { resolver }
    }

    pub fn new_service<E>() -> impl NewService<
        Request = T,
        Response = (T, ConnectionInfo, TcpStream),
        Error = ConnectorError,
        InitError = E,
    > + Clone {
        || -> FutureResult<Connector<T>, E> { ok(Connector::default()) }
    }

    pub fn new_service_with_config<E>(
        cfg: ResolverConfig, opts: ResolverOpts,
    ) -> impl NewService<
        Request = T,
        Response = (T, ConnectionInfo, TcpStream),
        Error = ConnectorError,
        InitError = E,
    > + Clone {
        move || -> FutureResult<Connector<T>, E> { ok(Connector::new(cfg.clone(), opts)) }
    }

    pub fn change_request<T2: HostAware>(&self) -> Connector<T2> {
        Connector {
            resolver: self.resolver.change_request(),
        }
    }
}

impl<T> Clone for Connector<T> {
    fn clone(&self) -> Self {
        Connector {
            resolver: self.resolver.clone(),
        }
    }
}

impl<T: HostAware> Service for Connector<T> {
    type Request = T;
    type Response = (T, ConnectionInfo, TcpStream);
    type Error = ConnectorError;
    type Future = ConnectorFuture<T>;

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

pub struct ConnectorFuture<T: HostAware> {
    fut: ResolverFuture<T>,
    fut2: Option<TcpConnector<T>>,
}

impl<T: HostAware> Future for ConnectorFuture<T> {
    type Item = (T, ConnectionInfo, TcpStream);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return fut.poll();
        }
        match self.fut.poll().map_err(ConnectorError::from)? {
            Async::Ready((req, host, addrs)) => {
                if addrs.is_empty() {
                    Err(ConnectorError::NoRecords)
                } else {
                    self.fut2 = Some(TcpConnector::new(req, host, addrs));
                    self.poll()
                }
            }
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

#[derive(Clone)]
pub struct DefaultConnector<T: HostAware>(Connector<T>);

impl<T: HostAware> Default for DefaultConnector<T> {
    fn default() -> Self {
        DefaultConnector(Connector::default())
    }
}

impl<T: HostAware> DefaultConnector<T> {
    pub fn new(cfg: ResolverConfig, opts: ResolverOpts) -> Self {
        DefaultConnector(Connector::new(cfg, opts))
    }
}

impl<T: HostAware> Service for DefaultConnector<T> {
    type Request = T;
    type Response = TcpStream;
    type Error = ConnectorError;
    type Future = DefaultConnectorFuture<T>;

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
pub struct DefaultConnectorFuture<T: HostAware> {
    fut: ConnectorFuture<T>,
}

impl<T: HostAware> Future for DefaultConnectorFuture<T> {
    type Item = TcpStream;
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready(try_ready!(self.fut.poll()).2))
    }
}

/// Tcp stream connector
pub struct TcpConnector<T> {
    req: Option<T>,
    host: Option<String>,
    addr: Option<SocketAddr>,
    addrs: VecDeque<SocketAddr>,
    stream: Option<ConnectFuture>,
}

impl<T> TcpConnector<T> {
    pub fn new(req: T, host: String, addrs: VecDeque<SocketAddr>) -> TcpConnector<T> {
        TcpConnector {
            addrs,
            req: Some(req),
            host: Some(host),
            addr: None,
            stream: None,
        }
    }
}

impl<T> Future for TcpConnector<T> {
    type Item = (T, ConnectionInfo, TcpStream);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // connect
        loop {
            if let Some(new) = self.stream.as_mut() {
                match new.poll() {
                    Ok(Async::Ready(sock)) => {
                        return Ok(Async::Ready((
                            self.req.take().unwrap(),
                            ConnectionInfo {
                                host: self.host.take().unwrap(),
                                addr: self.addr.take().unwrap(),
                            },
                            sock,
                        )))
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

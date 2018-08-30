use std::collections::VecDeque;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;

use futures::{
    future::{ok, FutureResult},
    Async, Future, Poll,
};
use tokio;
use tokio_tcp::{ConnectFuture, TcpStream};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::lookup_ip::LookupIpFuture;
use trust_dns_resolver::system_conf::read_system_conf;
use trust_dns_resolver::{AsyncResolver, Background};

use super::{NewService, Service};

pub trait HostAware {
    fn host(&self) -> &str;
}

impl HostAware for String {
    fn host(&self) -> &str {
        self.as_ref()
    }
}

#[derive(Fail, Debug)]
pub enum ConnectorError {
    /// Failed to resolve the hostname
    #[fail(display = "Failed resolving hostname: {}", _0)]
    Resolver(String),

    /// Address is invalid
    #[fail(display = "Invalid input: {}", _0)]
    InvalidInput(&'static str),

    /// Connection io error
    #[fail(display = "{}", _0)]
    IoError(io::Error),
}

pub struct ConnectionInfo {
    pub host: String,
    pub addr: SocketAddr,
}

pub struct Connector<T = String> {
    resolver: AsyncResolver,
    req: PhantomData<T>,
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
        let (resolver, bg) = AsyncResolver::new(cfg, opts);
        tokio::spawn(bg);
        Connector {
            resolver,
            req: PhantomData,
        }
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
            resolver: self.resolver.clone(),
            req: PhantomData,
        }
    }
}

impl<T> Clone for Connector<T> {
    fn clone(&self) -> Self {
        Connector {
            resolver: self.resolver.clone(),
            req: PhantomData,
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
        let fut = ResolveFut::new(req, 0, &self.resolver);

        ConnectorFuture { fut, fut2: None }
    }
}

pub struct ConnectorFuture<T: HostAware> {
    fut: ResolveFut<T>,
    fut2: Option<TcpConnector<T>>,
}

impl<T: HostAware> Future for ConnectorFuture<T> {
    type Item = (T, ConnectionInfo, TcpStream);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return fut.poll();
        }
        match self.fut.poll()? {
            Async::Ready((req, host, addrs)) => {
                self.fut2 = Some(TcpConnector::new(req, host, addrs));
                self.poll()
            }
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

/// Resolver future
struct ResolveFut<T> {
    req: Option<T>,
    host: Option<String>,
    port: u16,
    lookup: Option<Background<LookupIpFuture>>,
    addrs: Option<VecDeque<SocketAddr>>,
    error: Option<ConnectorError>,
    error2: Option<String>,
}

impl<T: HostAware> ResolveFut<T> {
    pub fn new(addr: T, port: u16, resolver: &AsyncResolver) -> Self {
        // we need to do dns resolution
        match ResolveFut::<T>::parse(addr.host(), port) {
            Ok((host, port)) => {
                let lookup = Some(resolver.lookup_ip(host.as_str()));
                ResolveFut {
                    port,
                    lookup,
                    req: Some(addr),
                    host: Some(host),
                    addrs: None,
                    error: None,
                    error2: None,
                }
            }
            Err(err) => ResolveFut {
                port,
                req: None,
                host: None,
                lookup: None,
                addrs: None,
                error: Some(err),
                error2: None,
            },
        }
    }

    fn parse(addr: &str, port: u16) -> Result<(String, u16), ConnectorError> {
        macro_rules! try_opt {
            ($e:expr, $msg:expr) => {
                match $e {
                    Some(r) => r,
                    None => return Err(ConnectorError::InvalidInput($msg)),
                }
            };
        }

        // split the string by ':' and convert the second part to u16
        let mut parts_iter = addr.splitn(2, ':');
        let host = try_opt!(parts_iter.next(), "invalid socket address");
        let port_str = parts_iter.next().unwrap_or("");
        let port: u16 = port_str.parse().unwrap_or(port);

        Ok((host.to_owned(), port))
    }
}

impl<T> Future for ResolveFut<T> {
    type Item = (T, String, VecDeque<SocketAddr>);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(err) = self.error.take() {
            Err(err)
        } else if let Some(err) = self.error2.take() {
            Err(ConnectorError::Resolver(err))
        } else if let Some(addrs) = self.addrs.take() {
            Ok(Async::Ready((
                self.req.take().unwrap(),
                self.host.take().unwrap(),
                addrs,
            )))
        } else {
            match self.lookup.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(ips)) => {
                    let addrs: VecDeque<_> = ips
                        .iter()
                        .map(|ip| SocketAddr::new(ip, self.port))
                        .collect();
                    if addrs.is_empty() {
                        Err(ConnectorError::Resolver(
                            "Expect at least one A dns record".to_owned(),
                        ))
                    } else {
                        Ok(Async::Ready((
                            self.req.take().unwrap(),
                            self.host.take().unwrap(),
                            addrs,
                        )))
                    }
                }
                Err(err) => Err(ConnectorError::Resolver(format!("{}", err))),
            }
        }
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

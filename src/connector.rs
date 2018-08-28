use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;

use futures::{future::ok, Async, Future, Poll};
use tokio;
use tokio_tcp::{ConnectFuture, TcpStream};
use tower_service::Service;
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::lookup_ip::LookupIpFuture;
use trust_dns_resolver::{AsyncResolver, Background};

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

pub struct Connector {
    resolver: AsyncResolver,
}

impl Connector {
    pub fn new() -> Self {
        let resolver = match AsyncResolver::from_system_conf() {
            Ok((resolver, bg)) => {
                tokio::spawn(bg);
                resolver
            }
            Err(err) => {
                warn!("Can not create system dns resolver: {}", err);
                let (resolver, bg) =
                    AsyncResolver::new(ResolverConfig::default(), ResolverOpts::default());
                tokio::spawn(bg);
                resolver
            }
        };

        Connector { resolver }
    }

    pub fn new_service<E>() -> impl Future<Item = Connector, Error = E> {
        ok(Connector::new())
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
    type Request = String;
    type Response = (String, TcpStream);
    type Error = ConnectorError;
    type Future = ConnectorFuture;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, addr: String) -> Self::Future {
        let fut = ResolveFut::new(&addr, 0, &self.resolver);

        ConnectorFuture {
            fut,
            addr: Some(addr),
            fut2: None,
        }
    }
}

pub struct ConnectorFuture {
    addr: Option<String>,
    fut: ResolveFut,
    fut2: Option<TcpConnector>,
}

impl Future for ConnectorFuture {
    type Item = (String, TcpStream);
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut2 {
            return match fut.poll()? {
                Async::Ready(stream) => Ok(Async::Ready((self.addr.take().unwrap(), stream))),
                Async::NotReady => Ok(Async::NotReady),
            };
        }
        match self.fut.poll()? {
            Async::Ready(addrs) => {
                self.fut2 = Some(TcpConnector::new(addrs));
                self.poll()
            }
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

/// Resolver future
struct ResolveFut {
    lookup: Option<Background<LookupIpFuture>>,
    port: u16,
    addrs: Option<VecDeque<SocketAddr>>,
    error: Option<ConnectorError>,
    error2: Option<String>,
}

impl ResolveFut {
    pub fn new(addr: &str, port: u16, resolver: &AsyncResolver) -> Self {
        // we need to do dns resolution
        match ResolveFut::parse(addr.as_ref(), port) {
            Ok((host, port)) => ResolveFut {
                port,
                lookup: Some(resolver.lookup_ip(host)),
                addrs: None,
                error: None,
                error2: None,
            },
            Err(err) => ResolveFut {
                port,
                lookup: None,
                addrs: None,
                error: Some(err),
                error2: None,
            },
        }
    }

    fn parse(addr: &str, port: u16) -> Result<(&str, u16), ConnectorError> {
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

        Ok((host, port))
    }
}

impl Future for ResolveFut {
    type Item = VecDeque<SocketAddr>;
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(err) = self.error.take() {
            Err(err)
        } else if let Some(err) = self.error2.take() {
            Err(ConnectorError::Resolver(err))
        } else if let Some(addrs) = self.addrs.take() {
            Ok(Async::Ready(addrs))
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
                        Ok(Async::Ready(addrs))
                    }
                }
                Err(err) => Err(ConnectorError::Resolver(format!("{}", err))),
            }
        }
    }
}

/// Tcp stream connector
pub struct TcpConnector {
    addrs: VecDeque<SocketAddr>,
    stream: Option<ConnectFuture>,
}

impl TcpConnector {
    pub fn new(addrs: VecDeque<SocketAddr>) -> TcpConnector {
        TcpConnector {
            addrs,
            stream: None,
        }
    }
}

impl Future for TcpConnector {
    type Item = TcpStream;
    type Error = ConnectorError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // connect
        loop {
            if let Some(new) = self.stream.as_mut() {
                match new.poll() {
                    Ok(Async::Ready(sock)) => return Ok(Async::Ready(sock)),
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
        }
    }
}

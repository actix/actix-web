use std::collections::VecDeque;
use std::marker::PhantomData;
use std::net::SocketAddr;

use futures::{Async, Future, Poll};

use tokio_current_thread::spawn;
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
pub use trust_dns_resolver::error::ResolveError;
use trust_dns_resolver::lookup_ip::LookupIpFuture;
use trust_dns_resolver::system_conf::read_system_conf;
use trust_dns_resolver::{AsyncResolver, Background};

use super::service::Service;

pub trait HostAware {
    fn host(&self) -> &str;
}

impl HostAware for String {
    fn host(&self) -> &str {
        self.as_ref()
    }
}

pub struct Resolver<T = String> {
    resolver: AsyncResolver,
    req: PhantomData<T>,
}

impl<T: HostAware> Default for Resolver<T> {
    fn default() -> Self {
        let (cfg, opts) = if let Ok((cfg, opts)) = read_system_conf() {
            (cfg, opts)
        } else {
            (ResolverConfig::default(), ResolverOpts::default())
        };

        Resolver::new(cfg, opts)
    }
}

impl<T: HostAware> Resolver<T> {
    pub fn new(cfg: ResolverConfig, opts: ResolverOpts) -> Self {
        let (resolver, bg) = AsyncResolver::new(cfg, opts);
        spawn(bg);
        Resolver {
            resolver,
            req: PhantomData,
        }
    }

    pub fn change_request<T2: HostAware>(&self) -> Resolver<T2> {
        Resolver {
            resolver: self.resolver.clone(),
            req: PhantomData,
        }
    }
}

impl<T> Clone for Resolver<T> {
    fn clone(&self) -> Self {
        Resolver {
            resolver: self.resolver.clone(),
            req: PhantomData,
        }
    }
}

impl<T: HostAware> Service for Resolver<T> {
    type Request = T;
    type Response = (T, VecDeque<SocketAddr>);
    type Error = ResolveError;
    type Future = ResolverFuture<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        ResolverFuture::new(req, &self.resolver)
    }
}

#[doc(hidden)]
/// Resolver future
pub struct ResolverFuture<T> {
    req: Option<T>,
    lookup: Option<Background<LookupIpFuture>>,
    addrs: Option<VecDeque<SocketAddr>>,
}

impl<T: HostAware> ResolverFuture<T> {
    pub fn new(addr: T, resolver: &AsyncResolver) -> Self {
        // we need to do dns resolution
        let lookup = Some(resolver.lookup_ip(addr.host()));
        ResolverFuture {
            lookup,
            req: Some(addr),
            addrs: None,
        }
    }
}

impl<T: HostAware> Future for ResolverFuture<T> {
    type Item = (T, VecDeque<SocketAddr>);
    type Error = ResolveError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(addrs) = self.addrs.take() {
            Ok(Async::Ready((self.req.take().unwrap(), addrs)))
        } else {
            match self.lookup.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(ips)) => {
                    let addrs: VecDeque<_> =
                        ips.iter().map(|ip| SocketAddr::new(ip, 0)).collect();
                    Ok(Async::Ready((self.req.take().unwrap(), addrs)))
                }
                Err(err) => Err(err),
            }
        }
    }
}

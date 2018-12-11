use std::collections::VecDeque;
use std::marker::PhantomData;
use std::net::IpAddr;

use actix_service::Service;
use futures::{Async, Future, Poll};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
pub use trust_dns_resolver::error::ResolveError;
use trust_dns_resolver::lookup_ip::LookupIpFuture;
use trust_dns_resolver::system_conf::read_system_conf;
use trust_dns_resolver::{AsyncResolver, Background};

/// Host name of the request
pub trait RequestHost {
    fn host(&self) -> &str;
}

impl RequestHost for String {
    fn host(&self) -> &str {
        self.as_ref()
    }
}

pub struct Resolver<T = String> {
    resolver: AsyncResolver,
    req: PhantomData<T>,
}

impl<T: RequestHost> Default for Resolver<T> {
    fn default() -> Self {
        let (cfg, opts) = if let Ok((cfg, opts)) = read_system_conf() {
            (cfg, opts)
        } else {
            (ResolverConfig::default(), ResolverOpts::default())
        };

        Resolver::new(cfg, opts)
    }
}

impl<T: RequestHost> Resolver<T> {
    /// Create new resolver instance with custom configuration and options.
    pub fn new(cfg: ResolverConfig, opts: ResolverOpts) -> Self {
        let (resolver, bg) = AsyncResolver::new(cfg, opts);
        actix_rt::Arbiter::spawn(bg);
        Resolver {
            resolver,
            req: PhantomData,
        }
    }

    /// Change type of resolver request.
    pub fn into_request<T2: RequestHost>(&self) -> Resolver<T2> {
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

impl<T: RequestHost> Service<T> for Resolver<T> {
    type Response = (T, VecDeque<IpAddr>);
    type Error = ResolveError;
    type Future = ResolverFuture<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: T) -> Self::Future {
        if let Ok(ip) = req.host().parse() {
            let mut addrs = VecDeque::new();
            addrs.push_back(ip);
            ResolverFuture::new(req, &self.resolver, Some(addrs))
        } else {
            ResolverFuture::new(req, &self.resolver, None)
        }
    }
}

#[doc(hidden)]
/// Resolver future
pub struct ResolverFuture<T> {
    req: Option<T>,
    lookup: Option<Background<LookupIpFuture>>,
    addrs: Option<VecDeque<IpAddr>>,
}

impl<T: RequestHost> ResolverFuture<T> {
    pub fn new(addr: T, resolver: &AsyncResolver, addrs: Option<VecDeque<IpAddr>>) -> Self {
        // we need to do dns resolution
        let lookup = Some(resolver.lookup_ip(addr.host()));
        ResolverFuture {
            lookup,
            addrs,
            req: Some(addr),
        }
    }
}

impl<T: RequestHost> Future for ResolverFuture<T> {
    type Item = (T, VecDeque<IpAddr>);
    type Error = ResolveError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(addrs) = self.addrs.take() {
            Ok(Async::Ready((self.req.take().unwrap(), addrs)))
        } else {
            match self.lookup.as_mut().unwrap().poll() {
                Ok(Async::NotReady) => Ok(Async::NotReady),
                Ok(Async::Ready(ips)) => Ok(Async::Ready((
                    self.req.take().unwrap(),
                    ips.iter().collect(),
                ))),
                Err(err) => Err(err),
            }
        }
    }
}

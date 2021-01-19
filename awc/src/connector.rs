// TODO: this mod bypass actix-http and use actix_tls::connect directly.
// Future refactor should change this mod if actix-http still used as upstream dep of awc.

pub use connector_impl::*;

#[cfg(not(feature = "trust-dns"))]
mod connector_impl {
    pub use actix_tls::connect::default_connector;
}

#[cfg(feature = "trust-dns")]
mod connector_impl {
    // resolver implementation using trust-dns crate.
    use std::net::SocketAddr;

    use actix_rt::{net::TcpStream, Arbiter};
    use actix_service::Service;
    use actix_tls::connect::{
        Address, Connect, ConnectError, Connection, Resolve, Resolver,
    };
    use futures_core::future::LocalBoxFuture;
    use trust_dns_resolver::{
        config::{ResolverConfig, ResolverOpts},
        system_conf::read_system_conf,
        TokioAsyncResolver,
    };

    pub struct TrustDnsResolver {
        resolver: TokioAsyncResolver,
    }

    impl TrustDnsResolver {
        fn new() -> Self {
            // dns struct is cached in Arbiter thread local map.
            // so new client constructor can reuse the dns resolver on local thread.

            if Arbiter::contains_item::<TokioAsyncResolver>() {
                Arbiter::get_item(|item: &TokioAsyncResolver| TrustDnsResolver {
                    resolver: item.clone(),
                })
            } else {
                let (cfg, opts) = match read_system_conf() {
                    Ok((cfg, opts)) => (cfg, opts),
                    Err(e) => {
                        log::error!("TRust-DNS can not load system config: {}", e);
                        (ResolverConfig::default(), ResolverOpts::default())
                    }
                };

                let resolver = TokioAsyncResolver::tokio(cfg, opts).unwrap();
                Arbiter::set_item(resolver.clone());
                TrustDnsResolver { resolver }
            }
        }
    }

    impl Resolve for TrustDnsResolver {
        fn lookup<'a>(
            &'a self,
            host: &'a str,
            port: u16,
        ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn std::error::Error>>>
        {
            Box::pin(async move {
                let res = self
                    .resolver
                    .lookup_ip(host)
                    .await?
                    .iter()
                    .map(|ip| SocketAddr::new(ip, port))
                    .collect();
                Ok(res)
            })
        }
    }

    pub fn default_connector<T: Address + 'static>(
    ) -> impl Service<Connect<T>, Response = Connection<T, TcpStream>, Error = ConnectError>
           + Clone {
        actix_tls::connect::new_connector(Resolver::new_custom(TrustDnsResolver::new()))
    }
}

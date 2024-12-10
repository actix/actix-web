use std::net::IpAddr;

use actix_http::header::{HeaderName, FORWARDED};
use ipnet::{AddrParseError, IpNet};

/// TrustedProxies is a helper struct to manage trusted proxies and headers
///
/// This is used to determine if information from a request can be trusted or not.
///
/// By default, it trusts the following:
///   - IPV4 Loopback
///   - IPV4 Private Networks
///   - IPV6 Loopback
///   - IPV6 Private Networks
///
/// It also trusts the `FORWARDED` header by default.
///
/// # Example
/// ```
/// use actix_web::trusted_proxies::TrustedProxies;
/// use actix_web::{web, App, HttpResponse, HttpServer};
///
/// let mut trusted_proxies = TrustedProxies::new_local();
/// trusted_proxies.add_trusted_proxy("168.10.0.0/16").unwrap();
/// trusted_proxies.add_trusted_header("X-Forwarded-For".parse().unwrap());
///
/// HttpServer::new(|| {
///     App::new()
///         .service(web::resource("/").to(|| async { "hello world" }))
/// }).trusted_proxies(trusted_proxies);
/// ```
#[derive(Debug, Clone)]
pub struct TrustedProxies(Vec<IpNet>, Vec<HeaderName>);

impl Default for TrustedProxies {
    fn default() -> Self {
        Self::new_local()
    }
}
impl TrustedProxies {
    /// Create a new TrustedProxies instance with no trusted proxies or headers
    pub fn new() -> Self {
        Self(vec![], vec![])
    }

    /// Create a new TrustedProxies instance with local and private networks and FORWARDED header trusted
    pub fn new_local() -> Self {
        Self(
            vec![
                // IPV4 Loopback
                "127.0.0.0/8".parse().unwrap(),
                // IPV4 Private Networks
                "10.0.0.0/8".parse().unwrap(),
                "172.16.0.0/12".parse().unwrap(),
                "192.168.0.0/16".parse().unwrap(),
                // IPV6 Loopback
                "::1/128".parse().unwrap(),
                // IPV6 Private network
                "fd00::/8".parse().unwrap(),
            ],
            vec![FORWARDED],
        )
    }

    /// Add a trusted header to the list of trusted headers
    pub fn add_trusted_header(&mut self, header: HeaderName) {
        self.1.push(header);
    }

    /// Add a trusted proxy to the list of trusted proxies
    ///
    /// proxy can be an IP address or a CIDR
    pub fn add_trusted_proxy(&mut self, proxy: &str) -> Result<(), AddrParseError> {
        match proxy.parse() {
            Ok(v) => {
                self.0.push(v);

                Ok(())
            }
            Err(e) => match proxy.parse::<IpAddr>() {
                Ok(v) => {
                    self.0.push(IpNet::from(v));

                    Ok(())
                }
                _ => Err(e),
            },
        }
    }

    /// Check if a remote address is trusted given the list of trusted proxies
    pub fn trust_ip(&self, remote_addr: &IpAddr) -> bool {
        for proxy in &self.0 {
            if proxy.contains(remote_addr) {
                return true;
            }
        }

        false
    }

    /// Check if a header is trusted
    pub fn trust_header(&self, header: &HeaderName) -> bool {
        self.1.contains(header)
    }
}

use std::{cell::Ref, convert::Infallible, net::SocketAddr};

use actix_utils::future::{err, ok, Ready};
use derive_more::{Display, Error};

use crate::{
    dev::{AppConfig, Payload, RequestHead},
    http::header::{self, HeaderName},
    FromRequest, HttpRequest, ResponseError,
};

const X_FORWARDED_FOR: &[u8] = b"x-forwarded-for";
const X_FORWARDED_HOST: &[u8] = b"x-forwarded-host";
const X_FORWARDED_PROTO: &[u8] = b"x-forwarded-proto";

/// `HttpRequest` connection information
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    scheme: String,
    host: String,
    realip_remote_addr: Option<String>,
    remote_addr: Option<String>,
}

impl ConnectionInfo {
    /// Create *ConnectionInfo* instance for a request.
    pub fn get<'a>(req: &'a RequestHead, cfg: &AppConfig) -> Ref<'a, Self> {
        if !req.extensions().contains::<ConnectionInfo>() {
            req.extensions_mut().insert(ConnectionInfo::new(req, cfg));
        }
        Ref::map(req.extensions(), |e| e.get().unwrap())
    }

    #[allow(clippy::cognitive_complexity, clippy::borrow_interior_mutable_const)]
    fn new(req: &RequestHead, cfg: &AppConfig) -> ConnectionInfo {
        let mut host = None;
        let mut scheme = None;
        let mut realip_remote_addr = None;

        // load forwarded header
        for hdr in req.headers.get_all(&header::FORWARDED) {
            if let Ok(val) = hdr.to_str() {
                for pair in val.split(';') {
                    for el in pair.split(',') {
                        let mut items = el.trim().splitn(2, '=');
                        if let Some(name) = items.next() {
                            if let Some(val) = items.next() {
                                match &name.to_lowercase() as &str {
                                    "for" => {
                                        if realip_remote_addr.is_none() {
                                            realip_remote_addr = Some(val.trim());
                                        }
                                    }
                                    "proto" => {
                                        if scheme.is_none() {
                                            scheme = Some(val.trim());
                                        }
                                    }
                                    "host" => {
                                        if host.is_none() {
                                            host = Some(val.trim());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // scheme
        if scheme.is_none() {
            if let Some(h) = req
                .headers
                .get(&HeaderName::from_lowercase(X_FORWARDED_PROTO).unwrap())
            {
                if let Ok(h) = h.to_str() {
                    scheme = h.split(',').next().map(|v| v.trim());
                }
            }
            if scheme.is_none() {
                scheme = req.uri.scheme().map(|a| a.as_str());
                if scheme.is_none() && cfg.secure() {
                    scheme = Some("https")
                }
            }
        }

        // host
        if host.is_none() {
            if let Some(h) = req
                .headers
                .get(&HeaderName::from_lowercase(X_FORWARDED_HOST).unwrap())
            {
                if let Ok(h) = h.to_str() {
                    host = h.split(',').next().map(|v| v.trim());
                }
            }
            if host.is_none() {
                if let Some(h) = req.headers.get(&header::HOST) {
                    host = h.to_str().ok();
                }
                if host.is_none() {
                    host = req.uri.authority().map(|a| a.as_str());
                    if host.is_none() {
                        host = Some(cfg.host());
                    }
                }
            }
        }

        // get remote_addraddr from socketaddr
        let remote_addr = req.peer_addr.map(|addr| format!("{}", addr));

        if realip_remote_addr.is_none() {
            if let Some(h) = req
                .headers
                .get(&HeaderName::from_lowercase(X_FORWARDED_FOR).unwrap())
            {
                if let Ok(h) = h.to_str() {
                    realip_remote_addr = h.split(',').next().map(|v| v.trim());
                }
            }
        }

        ConnectionInfo {
            remote_addr,
            scheme: scheme.unwrap_or("http").to_owned(),
            host: host.unwrap_or("localhost").to_owned(),
            realip_remote_addr: realip_remote_addr.map(|s| s.to_owned()),
        }
    }

    /// Scheme of the request.
    ///
    /// Scheme is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-Proto
    /// - Uri
    #[inline]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Hostname of the request.
    ///
    /// Hostname is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-Host
    /// - Host
    /// - Uri
    /// - Server hostname
    pub fn host(&self) -> &str {
        &self.host
    }

    /// remote_addr address of the request.
    ///
    /// Get remote_addr address from socket address
    pub fn remote_addr(&self) -> Option<&str> {
        if let Some(ref remote_addr) = self.remote_addr {
            Some(remote_addr)
        } else {
            None
        }
    }
    /// Real ip remote addr of client initiated HTTP request.
    ///
    /// The addr is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-For
    /// - remote_addr name of opened socket
    ///
    /// # Security
    /// Do not use this function for security purposes, unless you can ensure the Forwarded and
    /// X-Forwarded-For headers cannot be spoofed by the client. If you want the client's socket
    /// address explicitly, use
    /// [`HttpRequest::peer_addr()`](super::web::HttpRequest::peer_addr()) instead.
    #[inline]
    pub fn realip_remote_addr(&self) -> Option<&str> {
        if let Some(ref r) = self.realip_remote_addr {
            Some(r)
        } else if let Some(ref remote_addr) = self.remote_addr {
            Some(remote_addr)
        } else {
            None
        }
    }
}

impl FromRequest for ConnectionInfo {
    type Error = Infallible;
    type Future = Ready<Result<ConnectionInfo, Infallible>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(req.connection_info().clone())
    }
}

/// Extractor for peer's socket address.
///
/// Also see [`HttpRequest::peer_addr`].
///
/// # Examples
/// ```
/// # use actix_web::Responder;
/// use actix_web::dev::PeerAddr;
///
/// async fn handler(peer_addr: PeerAddr) -> impl Responder {
///     let socket_addr = peer_addr.0;
///     socket_addr.to_string()
/// }
/// # let _svc = actix_web::web::to(handler);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Display)]
#[display(fmt = "{}", _0)]
pub struct PeerAddr(pub SocketAddr);

impl PeerAddr {
    /// Unwrap into inner `SocketAddr` value.
    pub fn into_inner(self) -> SocketAddr {
        self.0
    }
}

#[derive(Debug, Display, Error)]
#[display(fmt = "Missing peer address")]
pub struct MissingPeerAddr;

impl ResponseError for MissingPeerAddr {}

impl FromRequest for PeerAddr {
    type Error = MissingPeerAddr;
    type Future = Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        match req.peer_addr() {
            Some(addr) => ok(PeerAddr(addr)),
            None => {
                log::error!("Missing peer address.");
                err(MissingPeerAddr)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::TestRequest;

    #[test]
    fn test_forwarded() {
        let req = TestRequest::default().to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "localhost:8080");

        let req = TestRequest::default()
            .insert_header((
                header::FORWARDED,
                "for=192.0.2.60; proto=https; by=203.0.113.43; host=rust-lang.org",
            ))
            .to_http_request();

        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));

        let req = TestRequest::default()
            .insert_header((header::HOST, "rust-lang.org"))
            .to_http_request();

        let info = req.connection_info();
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.realip_remote_addr(), None);

        let req = TestRequest::default()
            .insert_header((X_FORWARDED_FOR, "192.0.2.60"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));

        let req = TestRequest::default()
            .insert_header((X_FORWARDED_HOST, "192.0.2.60"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.host(), "192.0.2.60");
        assert_eq!(info.realip_remote_addr(), None);

        let req = TestRequest::default()
            .insert_header((X_FORWARDED_PROTO, "https"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
    }

    #[actix_rt::test]
    async fn test_conn_info() {
        let req = TestRequest::default()
            .uri("http://actix.rs/")
            .to_http_request();
        let conn_info = ConnectionInfo::extract(&req).await.unwrap();
        assert_eq!(conn_info.scheme(), "http");
    }

    #[actix_rt::test]
    async fn test_peer_addr() {
        let addr = "127.0.0.1:8080".parse().unwrap();
        let req = TestRequest::default().peer_addr(addr).to_http_request();
        let peer_addr = PeerAddr::extract(&req).await.unwrap();
        assert_eq!(peer_addr, PeerAddr(addr));

        let req = TestRequest::default().to_http_request();
        let res = PeerAddr::extract(&req).await;
        assert!(res.is_err());
    }
}

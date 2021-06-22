use std::{cell::Ref, convert::Infallible, net::SocketAddr};

use actix_utils::future::{err, ok, Ready};
use derive_more::{Display, Error};

use crate::{
    dev::{AppConfig, Payload, RequestHead},
    http::{
        header,
        uri::{Authority, Scheme},
    },
    FromRequest, HttpRequest, ResponseError,
};

const X_FORWARDED_FOR: &str = "x-forwarded-for";
const X_FORWARDED_HOST: &str = "x-forwarded-host";
const X_FORWARDED_PROTO: &str = "x-forwarded-proto";

/// HTTP connection information.
///
/// `ConnectionInfo` implements `FromRequest` and can be extracted in handlers.
///
/// # Examples
/// ```
/// # use actix_web::{HttpResponse, Responder};
/// use actix_web::dev::ConnectionInfo;
///
/// async fn handler(conn: ConnectionInfo) -> impl Responder {
///     match conn.host() {
///         "actix.rs" => HttpResponse::Ok().body("Welcome!"),
///         "admin.actix.rs" => HttpResponse::Ok().body("Admin portal."),
///         _ => HttpResponse::NotFound().finish()
///     }
/// }
/// # let _svc = actix_web::web::to(handler);
/// ```
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

    fn new(req: &RequestHead, cfg: &AppConfig) -> ConnectionInfo {
        let mut host = None;
        let mut scheme = None;
        let mut realip_remote_addr = None;

        for el in req
            .headers
            .get_all(&header::FORWARDED)
            .into_iter()
            .filter_map(|hdr| hdr.to_str().ok())
            .flat_map(|val| val.split(';'))
            .flat_map(|pair| pair.split(','))
        {
            let mut items = el.trim().splitn(2, '=');
            if let (Some(name), Some(val)) = (items.next(), items.next()) {
                match name.to_lowercase().as_ref() {
                    "for" => {
                        realip_remote_addr.get_or_insert_with(|| val.trim());
                    }
                    "proto" => {
                        scheme.get_or_insert_with(|| val.trim());
                    }
                    "host" => {
                        host.get_or_insert_with(|| val.trim());
                    }
                    _ => {}
                }
            }
        }

        let first_header_value = |name| {
            let val = req
                .headers
                .get(name)?
                .to_str()
                .ok()?
                .split(',')
                .next()?
                .trim();
            Some(val)
        };

        let scheme = scheme
            .or_else(|| first_header_value(X_FORWARDED_PROTO))
            .or_else(|| req.uri.scheme().map(Scheme::as_str))
            .or_else(|| cfg.secure().then(|| "https"))
            .unwrap_or("http")
            .to_owned();

        let host = host
            .or_else(|| first_header_value(X_FORWARDED_HOST))
            .or_else(|| req.headers.get(&header::HOST)?.to_str().ok())
            .or_else(|| req.uri.authority().map(Authority::as_str))
            .unwrap_or(cfg.host())
            .to_owned();

        let realip_remote_addr = realip_remote_addr
            .or_else(|| first_header_value(X_FORWARDED_FOR))
            .map(str::to_owned);

        let remote_addr = req.peer_addr.map(|addr| format!("{}", addr));

        ConnectionInfo {
            remote_addr,
            scheme,
            host,
            realip_remote_addr,
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
    type Future = Ready<Result<Self, Self::Error>>;
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
#[non_exhaustive]
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

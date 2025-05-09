use std::{convert::Infallible, net::SocketAddr};

use actix_utils::future::{err, ok, Ready};
use derive_more::{Display, Error};

use crate::{
    dev::{AppConfig, Payload, RequestHead},
    http::{
        header::{self, HeaderName},
        uri::{Authority, Scheme},
    },
    FromRequest, HttpRequest, ResponseError,
};

static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
static X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
static X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

/// Trim whitespace then any quote marks.
fn unquote(val: &str) -> &str {
    val.trim().trim_start_matches('"').trim_end_matches('"')
}

/// Remove port and IPv6 square brackets from a peer specification.
fn bare_address(val: &str) -> &str {
    if val.starts_with('[') {
        val.split("]:")
            .next()
            .map(|s| s.trim_start_matches('[').trim_end_matches(']'))
            // this indicates that the IPv6 address is malformed so shouldn't
            // usually happen, but if it does, just return the original input
            .unwrap_or(val)
    } else {
        val.split(':').next().unwrap_or(val)
    }
}

/// Extracts and trims first value for given header name.
fn first_header_value<'a>(req: &'a RequestHead, name: &'_ HeaderName) -> Option<&'a str> {
    let hdr = req.headers.get(name)?.to_str().ok()?;
    let val = hdr.split(',').next()?.trim();
    Some(val)
}

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
///
/// # Implementation Notes
/// Parses `Forwarded` header information according to [RFC 7239][rfc7239] but does not try to
/// interpret the values for each property. As such, the getter methods on `ConnectionInfo` return
/// strings instead of IP addresses or other types to acknowledge that they may be
/// [obfuscated][rfc7239-63] or [unknown][rfc7239-62].
///
/// If the older, related headers are also present (eg. `X-Forwarded-For`), then `Forwarded`
/// is preferred.
///
/// [rfc7239]: https://datatracker.ietf.org/doc/html/rfc7239
/// [rfc7239-62]: https://datatracker.ietf.org/doc/html/rfc7239#section-6.2
/// [rfc7239-63]: https://datatracker.ietf.org/doc/html/rfc7239#section-6.3
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    host: String,
    scheme: String,
    peer_addr: Option<String>,
    realip_remote_addr: Option<String>,
}

impl ConnectionInfo {
    pub(crate) fn new(req: &RequestHead, cfg: &AppConfig) -> ConnectionInfo {
        let mut host = None;
        let mut scheme = None;
        let mut realip_remote_addr = None;

        for (name, val) in req
            .headers
            .get_all(&header::FORWARDED)
            .filter_map(|hdr| hdr.to_str().ok())
            // "for=1.2.3.4, for=5.6.7.8; scheme=https"
            .flat_map(|val| val.split(';'))
            // ["for=1.2.3.4, for=5.6.7.8", " scheme=https"]
            .flat_map(|vals| vals.split(','))
            // ["for=1.2.3.4", " for=5.6.7.8", " scheme=https"]
            .flat_map(|pair| {
                let mut items = pair.trim().splitn(2, '=');
                Some((items.next()?, items.next()?))
            })
        {
            // [(name , val      ), ...                                    ]
            // [("for", "1.2.3.4"), ("for", "5.6.7.8"), ("scheme", "https")]

            // taking the first value for each property is correct because spec states that first
            // "for" value is client and rest are proxies; multiple values other properties have
            // no defined semantics
            //
            // > In a chain of proxy servers where this is fully utilized, the first
            // > "for" parameter will disclose the client where the request was first
            // > made, followed by any subsequent proxy identifiers.
            // --- https://datatracker.ietf.org/doc/html/rfc7239#section-5.2

            match name.trim().to_lowercase().as_str() {
                "for" => realip_remote_addr.get_or_insert_with(|| bare_address(unquote(val))),
                "proto" => scheme.get_or_insert_with(|| unquote(val)),
                "host" => host.get_or_insert_with(|| unquote(val)),
                "by" => {
                    // TODO: implement https://datatracker.ietf.org/doc/html/rfc7239#section-5.1
                    continue;
                }
                _ => continue,
            };
        }

        let scheme = scheme
            .or_else(|| first_header_value(req, &X_FORWARDED_PROTO))
            .or_else(|| req.uri.scheme().map(Scheme::as_str))
            .or_else(|| Some("https").filter(|_| cfg.secure()))
            .unwrap_or("http")
            .to_owned();

        let host = host
            .or_else(|| first_header_value(req, &X_FORWARDED_HOST))
            .or_else(|| req.headers.get(&header::HOST)?.to_str().ok())
            .or_else(|| req.uri.authority().map(Authority::as_str))
            .unwrap_or_else(|| cfg.host())
            .to_owned();

        let realip_remote_addr = realip_remote_addr
            .or_else(|| first_header_value(req, &X_FORWARDED_FOR))
            .map(str::to_owned);

        let peer_addr = req.peer_addr.map(|addr| addr.ip().to_string());

        ConnectionInfo {
            host,
            scheme,
            peer_addr,
            realip_remote_addr,
        }
    }

    /// Real IP (remote address) of client that initiated request.
    ///
    /// The address is resolved through the following, in order:
    /// - `Forwarded` header
    /// - `X-Forwarded-For` header
    /// - peer address of opened socket (same as [`remote_addr`](Self::remote_addr))
    ///
    /// # Security
    /// Do not use this function for security purposes unless you can be sure that the `Forwarded`
    /// and `X-Forwarded-For` headers cannot be spoofed by the client. If you are running without a
    /// proxy then [obtaining the peer address](Self::peer_addr) would be more appropriate.
    #[inline]
    pub fn realip_remote_addr(&self) -> Option<&str> {
        self.realip_remote_addr
            .as_deref()
            .or(self.peer_addr.as_deref())
    }

    /// Returns serialized IP address of the peer connection.
    ///
    /// See [`HttpRequest::peer_addr`] for more details.
    #[inline]
    pub fn peer_addr(&self) -> Option<&str> {
        self.peer_addr.as_deref()
    }

    /// Hostname of the request.
    ///
    /// Hostname is resolved through the following, in order:
    /// - `Forwarded` header
    /// - `X-Forwarded-Host` header
    /// - `Host` header
    /// - request target / URI
    /// - configured server hostname
    #[inline]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Scheme of the request.
    ///
    /// Scheme is resolved through the following, in order:
    /// - `Forwarded` header
    /// - `X-Forwarded-Proto` header
    /// - request target / URI
    #[inline]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Renamed to `peer_addr`.")]
    pub fn remote_addr(&self) -> Option<&str> {
        self.peer_addr()
    }
}

impl FromRequest for ConnectionInfo {
    type Error = Infallible;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ok(req.connection_info().clone())
    }
}

/// Extractor for peer's socket address.
///
/// Also see [`HttpRequest::peer_addr`] and [`ConnectionInfo::peer_addr`].
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
#[display("{}", _0)]
pub struct PeerAddr(pub SocketAddr);

impl PeerAddr {
    /// Unwrap into inner `SocketAddr` value.
    pub fn into_inner(self) -> SocketAddr {
        self.0
    }
}

#[derive(Debug, Display, Error)]
#[non_exhaustive]
#[display("Missing peer address")]
pub struct MissingPeerAddr;

impl ResponseError for MissingPeerAddr {}

impl FromRequest for PeerAddr {
    type Error = MissingPeerAddr;
    type Future = Ready<Result<Self, Self::Error>>;

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

    const X_FORWARDED_FOR: &str = "x-forwarded-for";
    const X_FORWARDED_HOST: &str = "x-forwarded-host";
    const X_FORWARDED_PROTO: &str = "x-forwarded-proto";

    #[test]
    fn info_default() {
        let req = TestRequest::default().to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "localhost:8080");
    }

    #[test]
    fn host_header() {
        let req = TestRequest::default()
            .insert_header((header::HOST, "rust-lang.org"))
            .to_http_request();

        let info = req.connection_info();
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.realip_remote_addr(), None);
    }

    #[test]
    fn x_forwarded_for_header() {
        let req = TestRequest::default()
            .insert_header((X_FORWARDED_FOR, "192.0.2.60"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));
    }

    #[test]
    fn x_forwarded_host_header() {
        let req = TestRequest::default()
            .insert_header((X_FORWARDED_HOST, "192.0.2.60"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.host(), "192.0.2.60");
        assert_eq!(info.realip_remote_addr(), None);
    }

    #[test]
    fn x_forwarded_proto_header() {
        let req = TestRequest::default()
            .insert_header((X_FORWARDED_PROTO, "https"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
    }

    #[test]
    fn forwarded_header() {
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
            .insert_header((
                header::FORWARDED,
                "for=192.0.2.60; proto=https; by=203.0.113.43; host=rust-lang.org",
            ))
            .to_http_request();

        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));
    }

    #[test]
    fn forwarded_case_sensitivity() {
        let req = TestRequest::default()
            .insert_header((header::FORWARDED, "For=192.0.2.60"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));
    }

    #[test]
    fn forwarded_weird_whitespace() {
        let req = TestRequest::default()
            .insert_header((header::FORWARDED, "for= 1.2.3.4; proto= https"))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("1.2.3.4"));
        assert_eq!(info.scheme(), "https");

        let req = TestRequest::default()
            .insert_header((header::FORWARDED, "  for = 1.2.3.4  "))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("1.2.3.4"));
    }

    #[test]
    fn forwarded_for_quoted() {
        let req = TestRequest::default()
            .insert_header((header::FORWARDED, r#"for="192.0.2.60:8080""#))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));
    }

    #[test]
    fn forwarded_for_ipv6() {
        let req = TestRequest::default()
            .insert_header((header::FORWARDED, r#"for="[2001:db8:cafe::17]""#))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("2001:db8:cafe::17"));
    }

    #[test]
    fn forwarded_for_ipv6_with_port() {
        let req = TestRequest::default()
            .insert_header((header::FORWARDED, r#"for="[2001:db8:cafe::17]:4711""#))
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("2001:db8:cafe::17"));
    }

    #[test]
    fn forwarded_for_multiple() {
        let req = TestRequest::default()
            .insert_header((header::FORWARDED, "for=192.0.2.60, for=198.51.100.17"))
            .to_http_request();
        let info = req.connection_info();
        // takes the first value
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));
    }

    #[test]
    fn scheme_from_uri() {
        let req = TestRequest::get()
            .uri("https://actix.rs/test")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
    }

    #[test]
    fn host_from_uri() {
        let req = TestRequest::get()
            .uri("https://actix.rs/test")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.host(), "actix.rs");
    }

    #[test]
    fn host_from_server_hostname() {
        let mut req = TestRequest::get();
        req.set_server_hostname("actix.rs");
        let req = req.to_http_request();

        let info = req.connection_info();
        assert_eq!(info.host(), "actix.rs");
    }

    #[actix_rt::test]
    async fn conn_info_extract() {
        let req = TestRequest::default()
            .uri("https://actix.rs/test")
            .to_http_request();
        let conn_info = ConnectionInfo::extract(&req).await.unwrap();
        assert_eq!(conn_info.scheme(), "https");
        assert_eq!(conn_info.host(), "actix.rs");
    }

    #[actix_rt::test]
    async fn peer_addr_extract() {
        let req = TestRequest::default().to_http_request();
        let res = PeerAddr::extract(&req).await;
        assert!(res.is_err());

        let addr = "127.0.0.1:8080".parse().unwrap();
        let req = TestRequest::default().peer_addr(addr).to_http_request();
        let peer_addr = PeerAddr::extract(&req).await.unwrap();
        assert_eq!(peer_addr, PeerAddr(addr));
    }

    #[actix_rt::test]
    async fn remote_address() {
        let req = TestRequest::default().to_http_request();
        let res = ConnectionInfo::extract(&req).await.unwrap();
        assert!(res.peer_addr().is_none());

        let addr = "127.0.0.1:8080".parse().unwrap();
        let req = TestRequest::default().peer_addr(addr).to_http_request();
        let conn_info = ConnectionInfo::extract(&req).await.unwrap();
        assert_eq!(conn_info.peer_addr().unwrap(), "127.0.0.1");
    }

    #[actix_rt::test]
    async fn real_ip_from_socket_addr() {
        let req = TestRequest::default().to_http_request();
        let res = ConnectionInfo::extract(&req).await.unwrap();
        assert!(res.realip_remote_addr().is_none());

        let addr = "127.0.0.1:8080".parse().unwrap();
        let req = TestRequest::default().peer_addr(addr).to_http_request();
        let conn_info = ConnectionInfo::extract(&req).await.unwrap();
        assert_eq!(conn_info.realip_remote_addr().unwrap(), "127.0.0.1");
    }
}

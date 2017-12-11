use std::str::FromStr;
use http::header::{self, HeaderName};
use httprequest::HttpRequest;

const X_FORWARDED_FOR: &str = "X-FORWARDED-FOR";
const X_FORWARDED_HOST: &str = "X-FORWARDED-HOST";
const X_FORWARDED_PROTO: &str = "X-FORWARDED-PROTO";


/// `HttpRequest` connection information
///
/// While it is possible to create `ConnectionInfo` directly,
/// consider using `HttpRequest::load_connection_info()` which cache result.
pub struct ConnectionInfo<'a> {
    scheme: &'a str,
    host: &'a str,
    remote: Option<&'a str>,
    peer: Option<String>,
}

impl<'a> ConnectionInfo<'a> {

    /// Create *ConnectionInfo* instance for a request.
    #[cfg_attr(feature = "cargo-clippy", allow(cyclomatic_complexity))]
    pub fn new<S>(req: &'a HttpRequest<S>) -> ConnectionInfo<'a> {
        let mut host = None;
        let mut scheme = None;
        let mut remote = None;
        let mut peer = None;

        // load forwarded header
        for hdr in req.headers().get_all(header::FORWARDED) {
            if let Ok(val) = hdr.to_str() {
                for pair in val.split(';') {
                    for el in pair.split(',') {
                        let mut items = el.trim().splitn(2, '=');
                        if let Some(name) = items.next() {
                            if let Some(val) = items.next() {
                                match &name.to_lowercase() as &str {
                                    "for" => if remote.is_none() {
                                        remote = Some(val.trim());
                                    },
                                    "proto" => if scheme.is_none() {
                                        scheme = Some(val.trim());
                                    },
                                    "host" => if host.is_none() {
                                        host = Some(val.trim());
                                    },
                                    _ => (),
                                }
                            }
                        }
                    }
                }
            }
        }

        // scheme
        if scheme.is_none() {
            if let Some(h) = req.headers().get(
                HeaderName::from_str(X_FORWARDED_PROTO).unwrap()) {
                if let Ok(h) = h.to_str() {
                    scheme = h.split(',').next().map(|v| v.trim());
                }
            }
            if scheme.is_none() {
                scheme = req.uri().scheme_part().map(|a| a.as_str());
                if scheme.is_none() {
                    if let Some(router) = req.router() {
                        if router.server_settings().secure() {
                            scheme = Some("https")
                        }
                    }
                }
            }
        }

        // host
        if host.is_none() {
            if let Some(h) = req.headers().get(HeaderName::from_str(X_FORWARDED_HOST).unwrap()) {
                if let Ok(h) = h.to_str() {
                    host = h.split(',').next().map(|v| v.trim());
                }
            }
            if host.is_none() {
                if let Some(h) = req.headers().get(header::HOST) {
                    host = h.to_str().ok();
                }
                if host.is_none() {
                    host = req.uri().authority_part().map(|a| a.as_str());
                    if host.is_none() {
                        if let Some(router) = req.router() {
                            host = Some(router.server_settings().host());
                        }
                    }
                }
            }
        }

        // remote addr
        if remote.is_none() {
            if let Some(h) = req.headers().get(
                HeaderName::from_str(X_FORWARDED_FOR).unwrap()) {
                if let Ok(h) = h.to_str() {
                    remote = h.split(',').next().map(|v| v.trim());
                }
            }
            if remote.is_none() { // get peeraddr from socketaddr
                peer = req.peer_addr().map(|addr| format!("{}", addr));
            }
        }

        ConnectionInfo {
            scheme: scheme.unwrap_or("http"),
            host: host.unwrap_or("localhost"),
            remote: remote,
            peer: peer,
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
        self.scheme
    }

    /// Hostname of the request.
    ///
    /// Hostname is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-Host
    /// - Host
    /// - Uri
    pub fn host(&self) -> &str {
        self.host
    }

    /// Remote IP of client initiated HTTP request.
    ///
    /// The IP is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-For
    /// - peername of opened socket
    #[inline]
    pub fn remote(&self) -> Option<&str> {
        if let Some(r) = self.remote {
            Some(r)
        } else if let Some(ref peer) = self.peer {
            Some(peer)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::HeaderValue;

    #[test]
    fn test_forwarded() {
        let req = HttpRequest::default();
        let info = ConnectionInfo::new(&req);
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "localhost");

        let mut req = HttpRequest::default();
        req.headers_mut().insert(
            header::FORWARDED,
            HeaderValue::from_static(
                "for=192.0.2.60; proto=https; by=203.0.113.43; host=rust-lang.org"));

        let info = ConnectionInfo::new(&req);
        assert_eq!(info.scheme(), "https");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.remote(), Some("192.0.2.60"));

        let mut req = HttpRequest::default();
        req.headers_mut().insert(
            header::HOST, HeaderValue::from_static("rust-lang.org"));

        let info = ConnectionInfo::new(&req);
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.remote(), None);

        let mut req = HttpRequest::default();
        req.headers_mut().insert(
            HeaderName::from_str(X_FORWARDED_FOR).unwrap(), HeaderValue::from_static("192.0.2.60"));
        let info = ConnectionInfo::new(&req);
        assert_eq!(info.remote(), Some("192.0.2.60"));

        let mut req = HttpRequest::default();
        req.headers_mut().insert(
            HeaderName::from_str(X_FORWARDED_HOST).unwrap(), HeaderValue::from_static("192.0.2.60"));
        let info = ConnectionInfo::new(&req);
        assert_eq!(info.host(), "192.0.2.60");
        assert_eq!(info.remote(), None);

        let mut req = HttpRequest::default();
        req.headers_mut().insert(
            HeaderName::from_str(X_FORWARDED_PROTO).unwrap(), HeaderValue::from_static("https"));
        let info = ConnectionInfo::new(&req);
        assert_eq!(info.scheme(), "https");
    }
}

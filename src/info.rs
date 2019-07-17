use std::cell::Ref;

use crate::dev::{AppConfig, RequestHead};
use crate::http::header::{self, HeaderName};

const X_FORWARDED_FOR: &[u8] = b"x-forwarded-for";
const X_FORWARDED_HOST: &[u8] = b"x-forwarded-host";
const X_FORWARDED_PROTO: &[u8] = b"x-forwarded-proto";

/// `HttpRequest` connection information
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    scheme: String,
    host: String,
    remote: Option<String>,
    peer: Option<String>,
}

impl ConnectionInfo {
    /// Create *ConnectionInfo* instance for a request.
    pub fn get<'a>(req: &'a RequestHead, cfg: &AppConfig) -> Ref<'a, Self> {
        if !req.extensions().contains::<ConnectionInfo>() {
            req.extensions_mut().insert(ConnectionInfo::new(req, cfg));
        }
        Ref::map(req.extensions(), |e| e.get().unwrap())
    }

    #[allow(clippy::cognitive_complexity)]
    fn new(req: &RequestHead, cfg: &AppConfig) -> ConnectionInfo {
        let mut host = None;
        let mut scheme = None;
        let mut remote = None;
        let mut peer = None;

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
                                        if remote.is_none() {
                                            remote = Some(val.trim());
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
            if let Some(h) = req
                .headers
                .get(&HeaderName::from_lowercase(X_FORWARDED_PROTO).unwrap())
            {
                if let Ok(h) = h.to_str() {
                    scheme = h.split(',').next().map(|v| v.trim());
                }
            }
            if scheme.is_none() {
                scheme = req.uri.scheme_part().map(|a| a.as_str());
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
                    host = req.uri.authority_part().map(|a| a.as_str());
                    if host.is_none() {
                        host = Some(cfg.host());
                    }
                }
            }
        }

        // remote addr
        if remote.is_none() {
            if let Some(h) = req
                .headers
                .get(&HeaderName::from_lowercase(X_FORWARDED_FOR).unwrap())
            {
                if let Ok(h) = h.to_str() {
                    remote = h.split(',').next().map(|v| v.trim());
                }
            }
            if remote.is_none() {
                // get peeraddr from socketaddr
                peer = req.peer_addr.map(|addr| format!("{}", addr));
            }
        }

        ConnectionInfo {
            peer,
            scheme: scheme.unwrap_or("http").to_owned(),
            host: host.unwrap_or("localhost").to_owned(),
            remote: remote.map(|s| s.to_owned()),
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

    /// Remote IP of client initiated HTTP request.
    ///
    /// The IP is resolved through the following headers, in this order:
    ///
    /// - Forwarded
    /// - X-Forwarded-For
    /// - peer name of opened socket
    #[inline]
    pub fn remote(&self) -> Option<&str> {
        if let Some(ref r) = self.remote {
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
    use crate::test::TestRequest;

    #[test]
    fn test_forwarded() {
        let req = TestRequest::default().to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "localhost:8080");

        let req = TestRequest::default()
            .header(
                header::FORWARDED,
                "for=192.0.2.60; proto=https; by=203.0.113.43; host=rust-lang.org",
            )
            .to_http_request();

        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.remote(), Some("192.0.2.60"));

        let req = TestRequest::default()
            .header(header::HOST, "rust-lang.org")
            .to_http_request();

        let info = req.connection_info();
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.remote(), None);

        let req = TestRequest::default()
            .header(X_FORWARDED_FOR, "192.0.2.60")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.remote(), Some("192.0.2.60"));

        let req = TestRequest::default()
            .header(X_FORWARDED_HOST, "192.0.2.60")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.host(), "192.0.2.60");
        assert_eq!(info.remote(), None);

        let req = TestRequest::default()
            .header(X_FORWARDED_PROTO, "https")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
    }
}

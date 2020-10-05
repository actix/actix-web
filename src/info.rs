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
    /// [`HttpRequest::peer_addr()`](../web/struct.HttpRequest.html#method.peer_addr) instead.
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
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));

        let req = TestRequest::default()
            .header(header::HOST, "rust-lang.org")
            .to_http_request();

        let info = req.connection_info();
        assert_eq!(info.scheme(), "http");
        assert_eq!(info.host(), "rust-lang.org");
        assert_eq!(info.realip_remote_addr(), None);

        let req = TestRequest::default()
            .header(X_FORWARDED_FOR, "192.0.2.60")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.realip_remote_addr(), Some("192.0.2.60"));

        let req = TestRequest::default()
            .header(X_FORWARDED_HOST, "192.0.2.60")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.host(), "192.0.2.60");
        assert_eq!(info.realip_remote_addr(), None);

        let req = TestRequest::default()
            .header(X_FORWARDED_PROTO, "https")
            .to_http_request();
        let info = req.connection_info();
        assert_eq!(info.scheme(), "https");
    }
}

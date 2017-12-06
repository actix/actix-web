use std::str::FromStr;
use http::header::{self, HeaderName};
use httprequest::HttpRequest;

const X_FORWARDED_HOST: &str = "X-FORWARDED-HOST";
const X_FORWARDED_PROTO: &str = "X-FORWARDED-PROTO";


/// `HttpRequest` connection information
///
/// While it is possible to create `ConnectionInfo` directly,
/// consider using `HttpRequest::load_connection_info()` which cache result.
pub struct ConnectionInfo<'a> {
    scheme: &'a str,
    host: &'a str,
    remote: String,
    forwarded_for: Vec<&'a str>,
    forwarded_by: Vec<&'a str>,
}

impl<'a> ConnectionInfo<'a> {

    /// Create *ConnectionInfo* instance for a request.
    pub fn new<S>(req: &'a HttpRequest<S>) -> ConnectionInfo<'a> {
        let mut host = None;
        let mut scheme = None;
        let mut forwarded_for = Vec::new();
        let mut forwarded_by = Vec::new();

        // load forwarded header
        for hdr in req.headers().get_all(header::FORWARDED) {
            if let Ok(val) = hdr.to_str() {
                for pair in val.split(';') {
                    for el in pair.split(',') {
                        let mut items = el.splitn(1, '=');
                        if let Some(name) = items.next() {
                            if let Some(val) = items.next() {
                                match &name.to_lowercase() as &str {
                                    "for" => forwarded_for.push(val.trim()),
                                    "by" => forwarded_by.push(val.trim()),
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
                if let Some(a) = req.uri().scheme_part() {
                    scheme = Some(a.as_str())
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
                    if let Ok(h) = h.to_str() {
                        host = Some(h);
                    }
                }
                if host.is_none() {
                    if let Some(a) = req.uri().authority_part() {
                        host = Some(a.as_str())
                    }
                }
            }
        }

        ConnectionInfo {
            scheme: scheme.unwrap_or("http"),
            host: host.unwrap_or("localhost"),
            remote: String::new(),
            forwarded_for: forwarded_for,
            forwarded_by: forwarded_by,
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
    pub fn remote(&self) -> &str {
        &self.remote
    }

    /// List of the nodes making the request to the proxy.
    #[inline]
    pub fn forwarded_for(&self) -> &Vec<&str> {
        &self.forwarded_for
    }

    /// List of the user-agent facing interface of the proxies
    #[inline]
    pub fn forwarded_by(&self) -> &Vec<&str> {
        &self.forwarded_by
    }
}

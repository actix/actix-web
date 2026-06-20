//! HTTP proxy support for [`crate::Client`].
//!
//! # Configuring a Proxy Explicitly
//!
//! ```no_run
//! use awc::{Client, proxy::Proxy};
//!
//! # #[actix_rt::main]
//! # async fn main() {
//! let proxy = Proxy::new("http://proxy.example.com:8080");
//! let client = Client::builder().proxy(proxy).finish();
//! # }
//! ```
//!
//! # Automatic Environment-Variable Detection
//!
//! When no proxy is set explicitly and [`crate::ClientBuilder::no_proxy`] has not been
//! called, [`Client`](crate::Client) will read `HTTPS_PROXY` (or `https_proxy`) first, then
//! `HTTP_PROXY` (or `http_proxy`) from the process environment and use the first value it
//! finds.
//!
//! ```no_run
//! // HTTP_PROXY=http://proxy.example.com:8080 cargo run
//! use awc::Client;
//!
//! # #[actix_rt::main]
//! # async fn main() {
//! // Automatically picks up the env var:
//! let client = Client::new();
//! # }
//! ```

use std::{fmt, str::FromStr};

use actix_http::Uri;
use base64::prelude::*;
use http::header::HeaderValue;

/// An HTTP proxy configuration.
///
/// Can be constructed manually or read from the `HTTP_PROXY` / `HTTPS_PROXY`
/// environment variables via [`Proxy::from_env`].
#[derive(Clone)]
pub struct Proxy {
    /// Full URI of the proxy server (e.g. `http://proxy.corp:3128`).
    pub(crate) uri: Uri,

    /// Optional `Proxy-Authorization` header value (pre-formatted, ready to send).
    pub(crate) auth_header: Option<HeaderValue>,
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Proxy")
            .field("uri", &self.uri)
            .field(
                "auth_header",
                &self.auth_header.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl Proxy {
    /// Construct a proxy that connects through `uri`.
    ///
    /// # Panics
    /// Panics if `uri` cannot be parsed as a valid [`Uri`].
    pub fn new(uri: impl TryInto<Uri, Error = impl fmt::Debug>) -> Self {
        let uri = uri.try_into().expect("proxy URI must be valid");
        Self {
            uri,
            auth_header: None,
        }
    }

    /// Construct a proxy with HTTP Basic authentication credentials.
    ///
    /// The credentials are encoded once during construction so that no heap
    /// allocation occurs per-request.
    ///
    /// # Panics
    /// Panics if `uri` cannot be parsed as a valid [`Uri`].
    pub fn new_with_creds(
        uri: impl TryInto<Uri, Error = impl fmt::Debug>,
        username: &str,
        password: &str,
    ) -> Self {
        let raw = format!("{username}:{password}");
        let encoded = BASE64_STANDARD.encode(raw);
        let header_value = HeaderValue::from_str(&format!("Basic {encoded}"))
            .expect("base64-encoded credentials are always valid header values");

        Self {
            uri: uri.try_into().expect("proxy URI must be valid"),
            auth_header: Some(header_value),
        }
    }

    /// Read an HTTP proxy from environment variables.
    ///
    /// Checked in order (first match wins):
    /// 1. `HTTPS_PROXY`
    /// 2. `https_proxy`
    /// 3. `HTTP_PROXY`
    /// 4. `http_proxy`
    ///
    /// Returns `None` when none of the variables are set or when the value
    /// cannot be parsed as a [`Uri`].
    pub fn from_env() -> Option<Self> {
        let candidates = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"];

        for var in candidates {
            if let Ok(val) = std::env::var(var) {
                if let Ok(uri) = Uri::from_str(&val) {
                    log::debug!("awc: using proxy from env var {var}={val}");
                    return Some(Self {
                        uri,
                        auth_header: None,
                    });
                }
            }
        }

        None
    }

    /// Returns the proxy's URI.
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Returns the proxy hostname (and port if non-standard) as a `String`,
    /// suitable for the `Host` header of a `CONNECT` request.
    #[allow(dead_code)]
    pub(crate) fn host_port(&self) -> Option<String> {
        let host = self.uri.host()?;
        match self.uri.port_u16() {
            Some(port) => Some(format!("{host}:{port}")),
            None => Some(host.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_new_parses_uri() {
        let p = Proxy::new("http://proxy.example.com:8080");
        assert_eq!(p.uri.host(), Some("proxy.example.com"));
        assert_eq!(p.uri.port_u16(), Some(8080));
        assert!(p.auth_header.is_none());
    }

    #[test]
    fn proxy_with_credentials_encodes_header() {
        let p = Proxy::new_with_creds("http://proxy.example.com:3128", "alice", "s3cr3t");
        let hdr = p.auth_header.unwrap();
        // "alice:s3cr3t" => YWxpY2U6czNjcjN0
        assert_eq!(hdr.to_str().unwrap(), "Basic YWxpY2U6czNjcjN0");
    }

    #[test]
    fn proxy_from_env_none_when_absent() {
        // make sure the variable is not set in the test environment
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("http_proxy");
        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("https_proxy");
        assert!(Proxy::from_env().is_none());
    }

    #[test]
    fn proxy_from_env_reads_http_proxy() {
        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("https_proxy");
        std::env::set_var("HTTP_PROXY", "http://myproxy:3128");
        let p = Proxy::from_env().expect("should find HTTP_PROXY");
        assert_eq!(p.uri.host(), Some("myproxy"));
        std::env::remove_var("HTTP_PROXY");
    }

    #[test]
    fn proxy_from_env_https_takes_priority() {
        std::env::set_var("HTTP_PROXY", "http://http-proxy:3128");
        std::env::set_var("HTTPS_PROXY", "http://https-proxy:8080");
        let p = Proxy::from_env().expect("should find HTTPS_PROXY");
        assert_eq!(p.uri.host(), Some("https-proxy"));
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("HTTPS_PROXY");
    }

    #[test]
    fn proxy_host_port_formats_correctly() {
        let p = Proxy::new("http://proxy.local:9090");
        assert_eq!(p.host_port(), Some("proxy.local:9090".to_owned()));
    }
}

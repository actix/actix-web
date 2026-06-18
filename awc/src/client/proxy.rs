use std::env;

use http::Uri;

/// HTTP proxy configuration for the client connector.
///
/// Determines which proxy (if any) should be used for a given request URI,
/// based on explicit configuration or environment variables (`HTTP_PROXY`,
/// `HTTPS_PROXY`, `NO_PROXY`).
#[derive(Clone, Debug)]
pub struct ProxyConfig {
    http_proxy: Option<Uri>,
    https_proxy: Option<Uri>,
    no_proxy: Option<NoProxy>,
}

impl ProxyConfig {
    /// Creates a proxy configuration by reading standard environment variables.
    ///
    /// Reads `HTTP_PROXY` (or `http_proxy`), `HTTPS_PROXY` (or `https_proxy`),
    /// and `NO_PROXY` (or `no_proxy`).
    pub fn from_env() -> Self {
        let http_proxy = env_var_uri("HTTP_PROXY").or_else(|| env_var_uri("http_proxy"));
        let https_proxy = env_var_uri("HTTPS_PROXY").or_else(|| env_var_uri("https_proxy"));
        let no_proxy = env::var("NO_PROXY")
            .or_else(|_| env::var("no_proxy"))
            .ok()
            .map(|s| NoProxy::parse(&s));

        Self {
            http_proxy,
            https_proxy,
            no_proxy,
        }
    }

    /// Creates a proxy configuration with an explicit HTTP proxy URI.
    pub fn with_http_proxy(mut self, proxy: Uri) -> Self {
        self.http_proxy = Some(proxy);
        self
    }

    /// Creates a proxy configuration with an explicit HTTPS proxy URI.
    pub fn with_https_proxy(mut self, proxy: Uri) -> Self {
        self.https_proxy = Some(proxy);
        self
    }

    /// Sets the no-proxy list (comma-separated hostnames/domains).
    pub fn with_no_proxy(mut self, no_proxy: &str) -> Self {
        self.no_proxy = Some(NoProxy::parse(no_proxy));
        self
    }

    /// Returns the proxy URI to use for the given target URI, or `None` if no proxy applies.
    pub fn proxy_for_uri(&self, uri: &Uri) -> Option<&Uri> {
        if let Some(ref no_proxy) = self.no_proxy {
            if let Some(host) = uri.host() {
                if no_proxy.matches(host) {
                    return None;
                }
            }
        }

        match uri.scheme_str() {
            Some("https") | Some("wss") => self.https_proxy.as_ref(),
            _ => self.http_proxy.as_ref(),
        }
    }

    /// Returns `true` if any proxy is configured.
    pub fn has_proxy(&self) -> bool {
        self.http_proxy.is_some() || self.https_proxy.is_some()
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            http_proxy: None,
            https_proxy: None,
            no_proxy: None,
        }
    }
}

fn env_var_uri(name: &str) -> Option<Uri> {
    env::var(name)
        .ok()
        .and_then(|val| {
            if val.is_empty() {
                return None;
            }
            match val.parse::<Uri>() {
                Ok(uri) => Some(uri),
                Err(err) => {
                    log::warn!("Invalid proxy URI in {name}: {err}");
                    None
                }
            }
        })
}

/// Parsed representation of the `NO_PROXY` environment variable.
#[derive(Clone, Debug)]
struct NoProxy {
    entries: Vec<String>,
    match_all: bool,
}

impl NoProxy {
    fn parse(value: &str) -> Self {
        let trimmed = value.trim();
        if trimmed == "*" {
            return Self {
                entries: Vec::new(),
                match_all: true,
            };
        }

        let entries = trimmed
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        Self {
            entries,
            match_all: false,
        }
    }

    fn matches(&self, host: &str) -> bool {
        if self.match_all {
            return true;
        }

        let host = host.to_lowercase();

        self.entries.iter().any(|entry| {
            if entry.starts_with('.') {
                // ".example.com" matches "foo.example.com" and "example.com"
                host.ends_with(entry.as_str()) || host == entry[1..]
            } else {
                host == *entry || host.ends_with(&format!(".{entry}"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_wildcard() {
        let np = NoProxy::parse("*");
        assert!(np.matches("anything.example.com"));
        assert!(np.matches("localhost"));
    }

    #[test]
    fn no_proxy_exact_match() {
        let np = NoProxy::parse("localhost, example.com");
        assert!(np.matches("localhost"));
        assert!(np.matches("example.com"));
        assert!(!np.matches("other.com"));
    }

    #[test]
    fn no_proxy_subdomain_match() {
        let np = NoProxy::parse("example.com");
        assert!(np.matches("example.com"));
        assert!(np.matches("sub.example.com"));
        assert!(!np.matches("notexample.com"));
    }

    #[test]
    fn no_proxy_dot_prefix() {
        let np = NoProxy::parse(".example.com");
        assert!(np.matches("example.com"));
        assert!(np.matches("sub.example.com"));
        assert!(!np.matches("notexample.com"));
    }

    #[test]
    fn no_proxy_case_insensitive() {
        let np = NoProxy::parse("Example.COM");
        assert!(np.matches("example.com"));
        assert!(np.matches("EXAMPLE.COM"));
    }

    #[test]
    fn proxy_for_uri_http() {
        let config = ProxyConfig::default()
            .with_http_proxy("http://proxy:8080".parse().unwrap());
        let uri: Uri = "http://example.com/foo".parse().unwrap();
        assert!(config.proxy_for_uri(&uri).is_some());
    }

    #[test]
    fn proxy_for_uri_no_proxy() {
        let config = ProxyConfig::default()
            .with_http_proxy("http://proxy:8080".parse().unwrap())
            .with_no_proxy("example.com");
        let uri: Uri = "http://example.com/foo".parse().unwrap();
        assert!(config.proxy_for_uri(&uri).is_none());
    }

    #[test]
    fn proxy_for_uri_https() {
        let config = ProxyConfig::default()
            .with_https_proxy("http://proxy:8080".parse().unwrap());
        let uri: Uri = "https://example.com/foo".parse().unwrap();
        assert!(config.proxy_for_uri(&uri).is_some());
    }

    #[test]
    fn no_proxy_empty_string() {
        let np = NoProxy::parse("");
        assert!(!np.matches("anything"));
    }
}

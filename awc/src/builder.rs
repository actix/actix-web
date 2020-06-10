use std::cell::RefCell;
use std::convert::TryFrom;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use actix_http::client::{Connect as HttpConnect, ConnectError, Connection, Connector};
use actix_http::http::{self, header, Error as HttpError, HeaderMap, HeaderName};
use actix_service::Service;

use crate::connect::{Connect, ConnectorWrapper};
use crate::{Client, ClientConfig};

/// An HTTP Client builder
///
/// This type can be used to construct an instance of `Client` through a
/// builder-like pattern.
pub struct ClientBuilder {
    default_headers: bool,
    allow_redirects: bool,
    max_redirects: usize,
    max_http_version: Option<http::Version>,
    stream_window_size: Option<u32>,
    conn_window_size: Option<u32>,
    headers: HeaderMap,
    timeout: Option<Duration>,
    connector: Option<RefCell<Box<dyn Connect>>>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    pub fn new() -> Self {
        ClientBuilder {
            default_headers: true,
            allow_redirects: true,
            max_redirects: 10,
            headers: HeaderMap::new(),
            timeout: Some(Duration::from_secs(5)),
            connector: None,
            max_http_version: None,
            stream_window_size: None,
            conn_window_size: None,
        }
    }

    /// Use custom connector service.
    pub fn connector<T>(mut self, connector: T) -> Self
    where
        T: Service<Request = HttpConnect, Error = ConnectError> + 'static,
        T::Response: Connection,
        <T::Response as Connection>::Future: 'static,
        T::Future: 'static,
    {
        self.connector = Some(RefCell::new(Box::new(ConnectorWrapper(connector))));
        self
    }

    /// Set request timeout
    ///
    /// Request timeout is the total time before a response must be received.
    /// Default value is 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Disable request timeout.
    pub fn disable_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    /// Do not follow redirects.
    ///
    /// Redirects are allowed by default.
    pub fn disable_redirects(mut self) -> Self {
        self.allow_redirects = false;
        self
    }

    /// Maximum supported http major version
    /// Supported versions http/1.1, http/2
    pub fn max_http_version(mut self, val: http::Version) -> Self {
        self.max_http_version = Some(val);
        self
    }

    /// Indicates the initial window size (in octets) for
    /// HTTP2 stream-level flow control for received data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_window_size(mut self, size: u32) -> Self {
        self.stream_window_size = Some(size);
        self
    }

    /// Indicates the initial window size (in octets) for
    /// HTTP2 connection-level flow control for received data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_connection_window_size(mut self, size: u32) -> Self {
        self.conn_window_size = Some(size);
        self
    }

    /// Set max number of redirects.
    ///
    /// Max redirects is set to 10 by default.
    pub fn max_redirects(mut self, num: usize) -> Self {
        self.max_redirects = num;
        self
    }

    /// Do not add default request headers.
    /// By default `Date` and `User-Agent` headers are set.
    pub fn no_default_headers(mut self) -> Self {
        self.default_headers = false;
        self
    }

    /// Add default header. Headers added by this method
    /// get added to every request.
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: fmt::Debug + Into<HttpError>,
        V: header::IntoHeaderValue,
        V::Error: fmt::Debug,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into() {
                Ok(value) => {
                    self.headers.append(key, value);
                }
                Err(e) => log::error!("Header value error: {:?}", e),
            },
            Err(e) => log::error!("Header name error: {:?}", e),
        }
        self
    }

    /// Set client wide HTTP basic authorization header
    pub fn basic_auth<U>(self, username: U, password: Option<&str>) -> Self
    where
        U: fmt::Display,
    {
        let auth = match password {
            Some(password) => format!("{}:{}", username, password),
            None => format!("{}:", username),
        };
        self.header(
            header::AUTHORIZATION,
            format!("Basic {}", base64::encode(&auth)),
        )
    }

    /// Set client wide HTTP bearer authentication header
    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: fmt::Display,
    {
        self.header(header::AUTHORIZATION, format!("Bearer {}", token))
    }

    /// Finish build process and create `Client` instance.
    pub fn finish(self) -> Client {
        let connector = if let Some(connector) = self.connector {
            connector
        } else {
            let mut connector = Connector::new();
            if let Some(val) = self.max_http_version {
                connector = connector.max_http_version(val)
            };
            if let Some(val) = self.conn_window_size {
                connector = connector.initial_connection_window_size(val)
            };
            if let Some(val) = self.stream_window_size {
                connector = connector.initial_window_size(val)
            };
            RefCell::new(
                Box::new(ConnectorWrapper(connector.finish())) as Box<dyn Connect>
            )
        };
        let config = ClientConfig {
            headers: self.headers,
            timeout: self.timeout,
            connector,
        };
        Client(Rc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_basic_auth() {
        let client = ClientBuilder::new().basic_auth("username", Some("password"));
        assert_eq!(
            client
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6cGFzc3dvcmQ="
        );

        let client = ClientBuilder::new().basic_auth("username", None);
        assert_eq!(
            client
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6"
        );
    }

    #[test]
    fn client_bearer_auth() {
        let client = ClientBuilder::new().bearer_auth("someS3cr3tAutht0k3n");
        assert_eq!(
            client
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer someS3cr3tAutht0k3n"
        );
    }
}

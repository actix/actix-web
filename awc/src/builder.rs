use std::cell::RefCell;
use std::convert::TryFrom;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use actix_http::client::{Connect, ConnectError, Connection, Connector};
use actix_http::http::{header, Error as HttpError, HeaderMap, HeaderName};
use actix_service::Service;

use crate::connect::ConnectorWrapper;
use crate::{Client, ClientConfig};

/// An HTTP Client builder
///
/// This type can be used to construct an instance of `Client` through a
/// builder-like pattern.
pub struct ClientBuilder {
    config: ClientConfig,
    default_headers: bool,
    allow_redirects: bool,
    max_redirects: usize,
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
            config: ClientConfig {
                headers: HeaderMap::new(),
                timeout: Some(Duration::from_secs(5)),
                connector: RefCell::new(Box::new(ConnectorWrapper(
                    Connector::new().finish(),
                ))),
            },
        }
    }

    /// Use custom connector service.
    pub fn connector<T>(mut self, connector: T) -> Self
    where
        T: Service<Request = Connect, Error = ConnectError> + 'static,
        T::Response: Connection,
        <T::Response as Connection>::Future: 'static,
        T::Future: 'static,
    {
        self.config.connector = RefCell::new(Box::new(ConnectorWrapper(connector)));
        self
    }

    /// Set request timeout
    ///
    /// Request timeout is the total time before a response must be received.
    /// Default value is 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    /// Disable request timeout.
    pub fn disable_timeout(mut self) -> Self {
        self.config.timeout = None;
        self
    }

    /// Do not follow redirects.
    ///
    /// Redirects are allowed by default.
    pub fn disable_redirects(mut self) -> Self {
        self.allow_redirects = false;
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
                    self.config.headers.append(key, value);
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
        Client(Rc::new(self.config))
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
                .config
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
                .config
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
                .config
                .headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer someS3cr3tAutht0k3n"
        );
    }
}

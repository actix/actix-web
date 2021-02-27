use std::convert::TryFrom;
use std::fmt;
use std::net::IpAddr;
use std::rc::Rc;
use std::time::Duration;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_http::{
    client::{Connector, TcpConnect, TcpConnectError, TcpConnection},
    http::{self, header, Error as HttpError, HeaderMap, HeaderName, Uri},
};
use actix_rt::net::TcpStream;
use actix_service::Service;

use crate::connect::ConnectorWrapper;
use crate::{Client, ClientConfig};

/// An HTTP Client builder
///
/// This type can be used to construct an instance of `Client` through a
/// builder-like pattern.
pub struct ClientBuilder<T = (), U = ()> {
    default_headers: bool,
    max_http_version: Option<http::Version>,
    stream_window_size: Option<u32>,
    conn_window_size: Option<u32>,
    headers: HeaderMap,
    timeout: Option<Duration>,
    local_address: Option<IpAddr>,
    connector: Connector<T, U>,
}

impl ClientBuilder {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> ClientBuilder<
        impl Service<
                TcpConnect<Uri>,
                Response = TcpConnection<Uri, TcpStream>,
                Error = TcpConnectError,
            > + Clone,
        TcpStream,
    > {
        ClientBuilder {
            default_headers: true,
            headers: HeaderMap::new(),
            timeout: Some(Duration::from_secs(5)),
            local_address: None,
            connector: Connector::new(),
            max_http_version: None,
            stream_window_size: None,
            conn_window_size: None,
        }
    }
}

impl<S, Io> ClientBuilder<S, Io>
where
    S: Service<TcpConnect<Uri>, Response = TcpConnection<Uri, Io>, Error = TcpConnectError>
        + Clone
        + 'static,
    Io: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
{
    /// Use custom connector service.
    pub fn connector<S1, Io1>(self, connector: Connector<S1, Io1>) -> ClientBuilder<S1, Io1>
    where
        S1: Service<
                TcpConnect<Uri>,
                Response = TcpConnection<Uri, Io1>,
                Error = TcpConnectError,
            > + Clone
            + 'static,
        Io1: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
    {
        ClientBuilder {
            default_headers: self.default_headers,
            headers: self.headers,
            timeout: self.timeout,
            local_address: None,
            connector,
            max_http_version: self.max_http_version,
            stream_window_size: self.stream_window_size,
            conn_window_size: self.conn_window_size,
        }
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

    /// Set local IP Address the connector would use for establishing connection.
    pub fn local_address(mut self, addr: IpAddr) -> Self {
        self.local_address = Some(addr);
        self
    }

    /// Maximum supported HTTP major version.
    ///
    /// Supported versions are HTTP/1.1 and HTTP/2.
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
            Ok(key) => match value.try_into_value() {
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
    pub fn basic_auth<N>(self, username: N, password: Option<&str>) -> Self
    where
        N: fmt::Display,
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
        let mut connector = self.connector;

        if let Some(val) = self.max_http_version {
            connector = connector.max_http_version(val);
        };
        if let Some(val) = self.conn_window_size {
            connector = connector.initial_connection_window_size(val)
        };
        if let Some(val) = self.stream_window_size {
            connector = connector.initial_window_size(val)
        };
        if let Some(val) = self.local_address {
            connector = connector.local_address(val);
        }

        let config = ClientConfig {
            headers: self.headers,
            timeout: self.timeout,
            connector: Box::new(ConnectorWrapper::new(connector.finish())) as _,
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

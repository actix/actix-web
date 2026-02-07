use std::{fmt, net::IpAddr, rc::Rc, time::Duration};

use actix_http::{
    error::HttpError,
    header::{self, HeaderMap, HeaderName, TryIntoHeaderPair},
    Uri,
};
use actix_rt::net::{ActixStream, TcpStream};
use actix_service::{boxed, Service};
use base64::prelude::*;

use crate::{
    client::{
        ClientConfig, ConnectInfo, Connector, ConnectorService, TcpConnectError, TcpConnection,
    },
    connect::DefaultConnector,
    error::SendRequestError,
    middleware::{NestTransform, Redirect, Transform},
    Client, ConnectRequest, ConnectResponse,
};

/// An HTTP Client builder
///
/// This type can be used to construct an instance of `Client` through a
/// builder-like pattern.
pub struct ClientBuilder<S = (), M = ()> {
    max_http_version: Option<http::Version>,
    stream_window_size: Option<u32>,
    conn_window_size: Option<u32>,
    fundamental_headers: bool,
    default_headers: HeaderMap,
    timeout: Option<Duration>,
    connector: Connector<S>,
    middleware: M,
    local_address: Option<IpAddr>,
    max_redirects: u8,
}

impl ClientBuilder {
    /// Create a new ClientBuilder with default settings
    ///
    /// Note: If the `rustls-0_23` feature is enabled and neither `rustls-0_23-native-roots` nor
    /// `rustls-0_23-webpki-roots` are enabled, this ClientBuilder will build without TLS. In order
    /// to enable TLS in this scenario, a custom `Connector` _must_ be added to the builder before
    /// finishing construction.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> ClientBuilder<
        impl Service<
                ConnectInfo<Uri>,
                Response = TcpConnection<Uri, TcpStream>,
                Error = TcpConnectError,
            > + Clone,
        (),
    > {
        ClientBuilder {
            max_http_version: None,
            stream_window_size: None,
            conn_window_size: None,
            fundamental_headers: true,
            default_headers: HeaderMap::new(),
            timeout: Some(Duration::from_secs(5)),
            connector: Connector::new(),
            middleware: (),
            local_address: None,
            max_redirects: 10,
        }
    }
}

impl<S, Io, M> ClientBuilder<S, M>
where
    S: Service<ConnectInfo<Uri>, Response = TcpConnection<Uri, Io>, Error = TcpConnectError>
        + Clone
        + 'static,
    Io: ActixStream + fmt::Debug + 'static,
{
    /// Use custom connector service.
    pub fn connector<S1, Io1>(self, connector: Connector<S1>) -> ClientBuilder<S1, M>
    where
        S1: Service<ConnectInfo<Uri>, Response = TcpConnection<Uri, Io1>, Error = TcpConnectError>
            + Clone
            + 'static,
        Io1: ActixStream + fmt::Debug + 'static,
    {
        ClientBuilder {
            middleware: self.middleware,
            fundamental_headers: self.fundamental_headers,
            default_headers: self.default_headers,
            timeout: self.timeout,
            local_address: self.local_address,
            connector,
            max_http_version: self.max_http_version,
            stream_window_size: self.stream_window_size,
            conn_window_size: self.conn_window_size,
            max_redirects: self.max_redirects,
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

    /// Do not follow redirects.
    ///
    /// Redirects are allowed by default.
    pub fn disable_redirects(mut self) -> Self {
        self.max_redirects = 0;
        self
    }

    /// Set max number of redirects.
    ///
    /// Max redirects is set to 10 by default.
    pub fn max_redirects(mut self, num: u8) -> Self {
        self.max_redirects = num;
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

    /// Do not add fundamental default request headers.
    ///
    /// By default `Date` and `User-Agent` headers are set.
    pub fn no_default_headers(mut self) -> Self {
        self.fundamental_headers = false;
        self
    }

    /// Add default header.
    ///
    /// Headers added by this method get added to every request unless overridden by other methods.
    ///
    /// # Panics
    /// Panics if header name or value is invalid.
    pub fn add_default_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        match header.try_into_pair() {
            Ok((key, value)) => self.default_headers.append(key, value),
            Err(err) => panic!("Header error: {:?}", err.into()),
        }

        self
    }

    #[doc(hidden)]
    #[deprecated(since = "3.0.0", note = "Prefer `add_default_header((key, value))`.")]
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: fmt::Debug + Into<HttpError>,
        V: header::TryIntoHeaderValue,
        V::Error: fmt::Debug,
    {
        match HeaderName::try_from(key) {
            Ok(key) => match value.try_into_value() {
                Ok(value) => {
                    self.default_headers.append(key, value);
                }
                Err(err) => log::error!("Header value error: {:?}", err),
            },
            Err(err) => log::error!("Header name error: {:?}", err),
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
        self.add_default_header((
            header::AUTHORIZATION,
            format!("Basic {}", BASE64_STANDARD.encode(auth)),
        ))
    }

    /// Set client wide HTTP bearer authentication header
    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: fmt::Display,
    {
        self.add_default_header((header::AUTHORIZATION, format!("Bearer {}", token)))
    }

    /// Registers middleware, in the form of a middleware component (type), that runs during inbound
    /// and/or outbound processing in the request life-cycle (request -> response),
    /// modifying request/response as necessary, across all requests managed by the `Client`.
    pub fn wrap<S1, M1>(self, mw: M1) -> ClientBuilder<S, NestTransform<M, M1, S1, ConnectRequest>>
    where
        M: Transform<S1, ConnectRequest>,
        M1: Transform<M::Transform, ConnectRequest>,
    {
        ClientBuilder {
            middleware: NestTransform::new(self.middleware, mw),
            fundamental_headers: self.fundamental_headers,
            max_http_version: self.max_http_version,
            stream_window_size: self.stream_window_size,
            conn_window_size: self.conn_window_size,
            default_headers: self.default_headers,
            timeout: self.timeout,
            connector: self.connector,
            local_address: self.local_address,
            max_redirects: self.max_redirects,
        }
    }

    /// Finish build process and create `Client` instance.
    pub fn finish(self) -> Client
    where
        M: Transform<DefaultConnector<ConnectorService<S, Io>>, ConnectRequest> + 'static,
        M::Transform: Service<ConnectRequest, Response = ConnectResponse, Error = SendRequestError>,
    {
        let max_redirects = self.max_redirects;

        if max_redirects > 0 {
            self.wrap(Redirect::new().max_redirect_times(max_redirects))
                ._finish()
        } else {
            self._finish()
        }
    }

    fn _finish(self) -> Client
    where
        M: Transform<DefaultConnector<ConnectorService<S, Io>>, ConnectRequest> + 'static,
        M::Transform: Service<ConnectRequest, Response = ConnectResponse, Error = SendRequestError>,
    {
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

        let connector = DefaultConnector::new(connector.finish());
        let connector = boxed::rc_service(self.middleware.new_transform(connector));

        Client(ClientConfig {
            default_headers: Rc::new(self.default_headers),
            timeout: self.timeout,
            connector,
        })
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
                .default_headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic dXNlcm5hbWU6cGFzc3dvcmQ="
        );

        let client = ClientBuilder::new().basic_auth("username", None);
        assert_eq!(
            client
                .default_headers
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
                .default_headers
                .get(header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer someS3cr3tAutht0k3n"
        );
    }
}

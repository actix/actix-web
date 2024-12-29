use std::{
    fmt,
    future::Future,
    net::IpAddr,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

use actix_http::Protocol;
use actix_rt::{
    net::{ActixStream, TcpStream},
    time::{sleep, Sleep},
};
use actix_service::Service;
use actix_tls::connect::{
    ConnectError as TcpConnectError, ConnectInfo, Connection as TcpConnection,
    Connector as TcpConnector, Resolver,
};
use futures_core::{future::LocalBoxFuture, ready};
use http::Uri;
use pin_project_lite::pin_project;

use super::{
    config::ConnectorConfig,
    connection::{Connection, ConnectionIo},
    error::ConnectError,
    pool::ConnectionPool,
    Connect,
};

enum OurTlsConnector {
    #[allow(dead_code)] // only dead when no TLS feature is enabled
    None,

    #[cfg(feature = "openssl")]
    Openssl(actix_tls::connect::openssl::reexports::SslConnector),

    /// Provided because building the OpenSSL context on newer versions can be very slow.
    /// This prevents unnecessary calls to `.build()` while constructing the client connector.
    #[cfg(feature = "openssl")]
    #[allow(dead_code)] // false positive; used in build_tls
    OpensslBuilder(actix_tls::connect::openssl::reexports::SslConnectorBuilder),

    #[cfg(feature = "rustls-0_20")]
    #[allow(dead_code)] // false positive; used in build_tls
    Rustls020(std::sync::Arc<actix_tls::connect::rustls_0_20::reexports::ClientConfig>),

    #[cfg(feature = "rustls-0_21")]
    #[allow(dead_code)] // false positive; used in build_tls
    Rustls021(std::sync::Arc<actix_tls::connect::rustls_0_21::reexports::ClientConfig>),

    #[cfg(any(
        feature = "rustls-0_22-webpki-roots",
        feature = "rustls-0_22-native-roots",
    ))]
    #[allow(dead_code)] // false positive; used in build_tls
    Rustls022(std::sync::Arc<actix_tls::connect::rustls_0_22::reexports::ClientConfig>),

    #[cfg(feature = "rustls-0_23")]
    #[allow(dead_code)] // false positive; used in build_tls
    Rustls023(std::sync::Arc<actix_tls::connect::rustls_0_23::reexports::ClientConfig>),
}

/// Manages HTTP client network connectivity.
///
/// The `Connector` type uses a builder-like combinator pattern for service construction that
/// finishes by calling the `.finish()` method.
///
/// ```no_run
/// use std::time::Duration;
///
/// let connector = awc::Connector::new()
///      .timeout(Duration::from_secs(5))
///      .finish();
/// ```
pub struct Connector<T> {
    connector: T,
    config: ConnectorConfig,

    #[allow(dead_code)] // only dead when no TLS feature is enabled
    tls: OurTlsConnector,
}

impl Connector<()> {
    /// Create a new connector with default TLS settings
    ///
    /// # Panics
    ///
    /// - When the `rustls-0_23-webpki-roots` or `rustls-0_23-native-roots` features are enabled
    ///     and no default crypto provider has been loaded, this method will panic.
    /// - When the `rustls-0_23-native-roots` or `rustls-0_22-native-roots` features are enabled
    ///     and the runtime system has no native root certificates, this method will panic.
    #[allow(clippy::new_ret_no_self, clippy::let_unit_value)]
    pub fn new() -> Connector<
        impl Service<
                ConnectInfo<Uri>,
                Response = TcpConnection<Uri, TcpStream>,
                Error = actix_tls::connect::ConnectError,
            > + Clone,
    > {
        Connector {
            connector: TcpConnector::new(resolver::resolver()).service(),
            config: ConnectorConfig::default(),
            tls: Self::build_tls(vec![b"h2".to_vec(), b"http/1.1".to_vec()]),
        }
    }

    cfg_if::cfg_if! {
        if #[cfg(any(feature = "rustls-0_23-webpki-roots", feature = "rustls-0_23-native-roots"))] {
            /// Build TLS connector with Rustls v0.23, based on supplied ALPN protocols.
            ///
            /// Note that if other TLS crate features are enabled, Rustls v0.23 will be used.
            fn build_tls(protocols: Vec<Vec<u8>>) -> OurTlsConnector {
                use actix_tls::connect::rustls_0_23::{self, reexports::ClientConfig};

                cfg_if::cfg_if! {
                    if #[cfg(feature = "rustls-0_23-webpki-roots")] {
                        let certs = rustls_0_23::webpki_roots_cert_store();
                    } else if #[cfg(feature = "rustls-0_23-native-roots")] {
                        let certs = rustls_0_23::native_roots_cert_store().expect("Failed to find native root certificates");
                    }
                }

                let mut config = ClientConfig::builder()
                    .with_root_certificates(certs)
                    .with_no_client_auth();

                config.alpn_protocols = protocols;

                OurTlsConnector::Rustls023(std::sync::Arc::new(config))
            }
        } else if #[cfg(any(feature = "rustls-0_22-webpki-roots", feature = "rustls-0_22-native-roots"))] {
            /// Build TLS connector with Rustls v0.22, based on supplied ALPN protocols.
            fn build_tls(protocols: Vec<Vec<u8>>) -> OurTlsConnector {
                use actix_tls::connect::rustls_0_22::{self, reexports::ClientConfig};

                cfg_if::cfg_if! {
                    if #[cfg(feature = "rustls-0_22-webpki-roots")] {
                        let certs = rustls_0_22::webpki_roots_cert_store();
                    } else if #[cfg(feature = "rustls-0_22-native-roots")] {
                        let certs = rustls_0_22::native_roots_cert_store().expect("Failed to find native root certificates");
                    }
                }

                let mut config = ClientConfig::builder()
                    .with_root_certificates(certs)
                    .with_no_client_auth();

                config.alpn_protocols = protocols;

                OurTlsConnector::Rustls022(std::sync::Arc::new(config))
            }
        } else if #[cfg(feature = "rustls-0_21")] {
            /// Build TLS connector with Rustls v0.21, based on supplied ALPN protocols.
            fn build_tls(protocols: Vec<Vec<u8>>) -> OurTlsConnector {
                use actix_tls::connect::rustls_0_21::{reexports::ClientConfig, webpki_roots_cert_store};

                let mut config = ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(webpki_roots_cert_store())
                    .with_no_client_auth();

                config.alpn_protocols = protocols;

                OurTlsConnector::Rustls021(std::sync::Arc::new(config))
            }
        } else if #[cfg(feature = "rustls-0_20")] {
            /// Build TLS connector with Rustls v0.20, based on supplied ALPN protocols.
            fn build_tls(protocols: Vec<Vec<u8>>) -> OurTlsConnector {
                use actix_tls::connect::rustls_0_20::{reexports::ClientConfig, webpki_roots_cert_store};

                let mut config = ClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(webpki_roots_cert_store())
                    .with_no_client_auth();

                config.alpn_protocols = protocols;

                OurTlsConnector::Rustls020(std::sync::Arc::new(config))
            }
        } else if #[cfg(feature = "openssl")] {
            /// Build TLS connector with OpenSSL, based on supplied ALPN protocols.
            fn build_tls(protocols: Vec<Vec<u8>>) -> OurTlsConnector {
                use actix_tls::connect::openssl::reexports::{SslConnector, SslMethod};
                use bytes::{BufMut, BytesMut};

                let mut alpn = BytesMut::with_capacity(20);
                for proto in &protocols {
                    alpn.put_u8(proto.len() as u8);
                    alpn.put(proto.as_slice());
                }

                let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();
                if let Err(err) = ssl.set_alpn_protos(&alpn) {
                    log::error!("Can not set ALPN protocol: {err:?}");
                }

                OurTlsConnector::OpensslBuilder(ssl)
            }
        } else {
            /// Provides an empty TLS connector when no TLS feature is enabled, or when only the
            /// `rustls-0_23` crate feature is enabled.
            fn build_tls(_: Vec<Vec<u8>>) -> OurTlsConnector {
                OurTlsConnector::None
            }
        }
    }
}

impl<S> Connector<S> {
    /// Sets custom connector.
    pub fn connector<S1, Io1>(self, connector: S1) -> Connector<S1>
    where
        Io1: ActixStream + fmt::Debug + 'static,
        S1: Service<ConnectInfo<Uri>, Response = TcpConnection<Uri, Io1>, Error = TcpConnectError>
            + Clone,
    {
        Connector {
            connector,
            config: self.config,
            tls: self.tls,
        }
    }
}

impl<S, IO> Connector<S>
where
    // Note:
    // Input Io type is bound to ActixStream trait but internally in client module they
    // are bound to ConnectionIo trait alias. And latter is the trait exposed to public
    // in the form of Box<dyn ConnectionIo> type.
    //
    // This remap is to hide ActixStream's trait methods. They are not meant to be called
    // from user code.
    IO: ActixStream + fmt::Debug + 'static,
    S: Service<ConnectInfo<Uri>, Response = TcpConnection<Uri, IO>, Error = TcpConnectError>
        + Clone
        + 'static,
{
    /// Sets TCP connection timeout.
    ///
    /// This is the max time allowed to connect to remote host, including DNS name resolution.
    ///
    /// By default, the timeout is 5 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Sets TLS handshake timeout.
    ///
    /// This is the max time allowed to perform the TLS handshake with remote host after TCP
    /// connection is established.
    ///
    /// By default, the timeout is 5 seconds.
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.config.handshake_timeout = timeout;
        self
    }

    /// Sets custom OpenSSL `SslConnector` instance.
    #[cfg(feature = "openssl")]
    pub fn openssl(
        mut self,
        connector: actix_tls::connect::openssl::reexports::SslConnector,
    ) -> Self {
        self.tls = OurTlsConnector::Openssl(connector);
        self
    }

    /// See docs for [`Connector::openssl`].
    #[doc(hidden)]
    #[cfg(feature = "openssl")]
    #[deprecated(since = "3.0.0", note = "Renamed to `Connector::openssl`.")]
    pub fn ssl(mut self, connector: actix_tls::connect::openssl::reexports::SslConnector) -> Self {
        self.tls = OurTlsConnector::Openssl(connector);
        self
    }

    /// Sets custom Rustls v0.20 `ClientConfig` instance.
    #[cfg(feature = "rustls-0_20")]
    pub fn rustls(
        mut self,
        connector: std::sync::Arc<actix_tls::connect::rustls_0_20::reexports::ClientConfig>,
    ) -> Self {
        self.tls = OurTlsConnector::Rustls020(connector);
        self
    }

    /// Sets custom Rustls v0.21 `ClientConfig` instance.
    #[cfg(feature = "rustls-0_21")]
    pub fn rustls_021(
        mut self,
        connector: std::sync::Arc<actix_tls::connect::rustls_0_21::reexports::ClientConfig>,
    ) -> Self {
        self.tls = OurTlsConnector::Rustls021(connector);
        self
    }

    /// Sets custom Rustls v0.22 `ClientConfig` instance.
    #[cfg(any(
        feature = "rustls-0_22-webpki-roots",
        feature = "rustls-0_22-native-roots",
    ))]
    pub fn rustls_0_22(
        mut self,
        connector: std::sync::Arc<actix_tls::connect::rustls_0_22::reexports::ClientConfig>,
    ) -> Self {
        self.tls = OurTlsConnector::Rustls022(connector);
        self
    }

    /// Sets custom Rustls v0.23 `ClientConfig` instance.
    ///
    /// In order to enable ALPN, set the `.alpn_protocols` field on the ClientConfig to the
    /// following:
    ///
    /// ```no_run
    /// vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    /// # ;
    /// ```
    #[cfg(feature = "rustls-0_23")]
    pub fn rustls_0_23(
        mut self,
        connector: std::sync::Arc<actix_tls::connect::rustls_0_23::reexports::ClientConfig>,
    ) -> Self {
        self.tls = OurTlsConnector::Rustls023(connector);
        self
    }

    /// Sets maximum supported HTTP major version.
    ///
    /// Supported versions are HTTP/1.1 and HTTP/2.
    pub fn max_http_version(mut self, val: http::Version) -> Self {
        let versions = match val {
            http::Version::HTTP_11 => vec![b"http/1.1".to_vec()],
            http::Version::HTTP_2 => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            _ => {
                unimplemented!("actix-http client only supports versions http/1.1 & http/2")
            }
        };
        self.tls = Connector::build_tls(versions);
        self
    }

    /// Sets the initial window size (in bytes) for HTTP/2 stream-level flow control for received
    /// data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_window_size(mut self, size: u32) -> Self {
        self.config.stream_window_size = size;
        self
    }

    /// Sets the initial window size (in bytes) for HTTP/2 connection-level flow control for
    /// received data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_connection_window_size(mut self, size: u32) -> Self {
        self.config.conn_window_size = size;
        self
    }

    /// Set total number of simultaneous connections per type of scheme.
    ///
    /// If limit is 0, the connector has no limit.
    ///
    /// The default limit size is 100.
    pub fn limit(mut self, limit: usize) -> Self {
        if limit == 0 {
            self.config.limit = u32::MAX as usize;
        } else {
            self.config.limit = limit;
        }

        self
    }

    /// Set keep-alive period for opened connection.
    ///
    /// Keep-alive period is the period between connection usage. If
    /// the delay between repeated usages of the same connection
    /// exceeds this period, the connection is closed.
    /// Default keep-alive period is 15 seconds.
    pub fn conn_keep_alive(mut self, dur: Duration) -> Self {
        self.config.conn_keep_alive = dur;
        self
    }

    /// Set max lifetime period for connection.
    ///
    /// Connection lifetime is max lifetime of any opened connection
    /// until it is closed regardless of keep-alive period.
    /// Default lifetime period is 75 seconds.
    pub fn conn_lifetime(mut self, dur: Duration) -> Self {
        self.config.conn_lifetime = dur;
        self
    }

    /// Set server connection disconnect timeout in milliseconds.
    ///
    /// Defines a timeout for disconnect connection. If a disconnect procedure does not complete
    /// within this time, the socket get dropped. This timeout affects only secure connections.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default disconnect timeout is set to 3000 milliseconds.
    pub fn disconnect_timeout(mut self, dur: Duration) -> Self {
        self.config.disconnect_timeout = Some(dur);
        self
    }

    /// Set local IP Address the connector would use for establishing connection.
    pub fn local_address(mut self, addr: IpAddr) -> Self {
        self.config.local_address = Some(addr);
        self
    }

    /// Finish configuration process and create connector service.
    ///
    /// The `Connector` builder always concludes by calling `finish()` last in its combinator chain.
    pub fn finish(self) -> ConnectorService<S, IO> {
        let local_address = self.config.local_address;
        let timeout = self.config.timeout;

        let tcp_service_inner =
            TcpConnectorInnerService::new(self.connector, timeout, local_address);

        #[allow(clippy::redundant_clone)]
        let tcp_service = TcpConnectorService {
            service: tcp_service_inner.clone(),
        };

        let tls = match self.tls {
            #[cfg(feature = "openssl")]
            OurTlsConnector::OpensslBuilder(builder) => OurTlsConnector::Openssl(builder.build()),
            tls => tls,
        };

        let tls_service = match tls {
            OurTlsConnector::None => {
                #[cfg(not(feature = "dangerous-h2c"))]
                {
                    None
                }

                #[cfg(feature = "dangerous-h2c")]
                {
                    use std::io;

                    use actix_tls::connect::Connection;
                    use actix_utils::future::{ready, Ready};

                    #[allow(non_local_definitions)]
                    impl IntoConnectionIo for TcpConnection<Uri, Box<dyn ConnectionIo>> {
                        fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                            let io = self.into_parts().0;
                            (io, Protocol::Http2)
                        }
                    }

                    /// With the `dangerous-h2c` feature enabled, this connector uses a no-op TLS
                    /// connection service that passes through plain TCP as a TLS connection.
                    ///
                    /// The protocol version of this fake TLS connection is set to be HTTP/2.
                    #[derive(Clone)]
                    struct NoOpTlsConnectorService;

                    impl<R, IO> Service<Connection<R, IO>> for NoOpTlsConnectorService
                    where
                        IO: ActixStream + 'static,
                    {
                        type Response = Connection<R, Box<dyn ConnectionIo>>;
                        type Error = io::Error;
                        type Future = Ready<Result<Self::Response, Self::Error>>;

                        actix_service::always_ready!();

                        fn call(&self, connection: Connection<R, IO>) -> Self::Future {
                            let (io, connection) = connection.replace_io(());
                            let (_, connection) = connection.replace_io(Box::new(io) as _);

                            ready(Ok(connection))
                        }
                    }

                    let handshake_timeout = self.config.handshake_timeout;

                    let tls_service = TlsConnectorService {
                        tcp_service: tcp_service_inner,
                        tls_service: NoOpTlsConnectorService,
                        timeout: handshake_timeout,
                    };

                    Some(actix_service::boxed::rc_service(tls_service))
                }
            }

            #[cfg(feature = "openssl")]
            OurTlsConnector::Openssl(tls) => {
                const H2: &[u8] = b"h2";

                use actix_tls::connect::openssl::{reexports::AsyncSslStream, TlsConnector};

                #[allow(non_local_definitions)]
                impl<IO: ConnectionIo> IntoConnectionIo for TcpConnection<Uri, AsyncSslStream<IO>> {
                    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                        let sock = self.into_parts().0;
                        let h2 = sock
                            .ssl()
                            .selected_alpn_protocol()
                            .is_some_and(|protos| protos.windows(2).any(|w| w == H2));

                        if h2 {
                            (Box::new(sock), Protocol::Http2)
                        } else {
                            (Box::new(sock), Protocol::Http1)
                        }
                    }
                }

                let handshake_timeout = self.config.handshake_timeout;

                let tls_service = TlsConnectorService {
                    tcp_service: tcp_service_inner,
                    tls_service: TlsConnector::service(tls),
                    timeout: handshake_timeout,
                };

                Some(actix_service::boxed::rc_service(tls_service))
            }

            #[cfg(feature = "openssl")]
            OurTlsConnector::OpensslBuilder(_) => {
                unreachable!("OpenSSL builder is built before this match.");
            }

            #[cfg(feature = "rustls-0_20")]
            OurTlsConnector::Rustls020(tls) => {
                const H2: &[u8] = b"h2";

                use actix_tls::connect::rustls_0_20::{reexports::AsyncTlsStream, TlsConnector};

                #[allow(non_local_definitions)]
                impl<Io: ConnectionIo> IntoConnectionIo for TcpConnection<Uri, AsyncTlsStream<Io>> {
                    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                        let sock = self.into_parts().0;
                        let h2 = sock
                            .get_ref()
                            .1
                            .alpn_protocol()
                            .is_some_and(|protos| protos.windows(2).any(|w| w == H2));

                        if h2 {
                            (Box::new(sock), Protocol::Http2)
                        } else {
                            (Box::new(sock), Protocol::Http1)
                        }
                    }
                }

                let handshake_timeout = self.config.handshake_timeout;

                let tls_service = TlsConnectorService {
                    tcp_service: tcp_service_inner,
                    tls_service: TlsConnector::service(tls),
                    timeout: handshake_timeout,
                };

                Some(actix_service::boxed::rc_service(tls_service))
            }

            #[cfg(feature = "rustls-0_21")]
            OurTlsConnector::Rustls021(tls) => {
                const H2: &[u8] = b"h2";

                use actix_tls::connect::rustls_0_21::{reexports::AsyncTlsStream, TlsConnector};

                #[allow(non_local_definitions)]
                impl<Io: ConnectionIo> IntoConnectionIo for TcpConnection<Uri, AsyncTlsStream<Io>> {
                    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                        let sock = self.into_parts().0;
                        let h2 = sock
                            .get_ref()
                            .1
                            .alpn_protocol()
                            .is_some_and(|protos| protos.windows(2).any(|w| w == H2));

                        if h2 {
                            (Box::new(sock), Protocol::Http2)
                        } else {
                            (Box::new(sock), Protocol::Http1)
                        }
                    }
                }

                let handshake_timeout = self.config.handshake_timeout;

                let tls_service = TlsConnectorService {
                    tcp_service: tcp_service_inner,
                    tls_service: TlsConnector::service(tls),
                    timeout: handshake_timeout,
                };

                Some(actix_service::boxed::rc_service(tls_service))
            }

            #[cfg(any(
                feature = "rustls-0_22-webpki-roots",
                feature = "rustls-0_22-native-roots",
            ))]
            OurTlsConnector::Rustls022(tls) => {
                const H2: &[u8] = b"h2";

                use actix_tls::connect::rustls_0_22::{reexports::AsyncTlsStream, TlsConnector};

                #[allow(non_local_definitions)]
                impl<Io: ConnectionIo> IntoConnectionIo for TcpConnection<Uri, AsyncTlsStream<Io>> {
                    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                        let sock = self.into_parts().0;
                        let h2 = sock
                            .get_ref()
                            .1
                            .alpn_protocol()
                            .is_some_and(|protos| protos.windows(2).any(|w| w == H2));

                        if h2 {
                            (Box::new(sock), Protocol::Http2)
                        } else {
                            (Box::new(sock), Protocol::Http1)
                        }
                    }
                }

                let handshake_timeout = self.config.handshake_timeout;

                let tls_service = TlsConnectorService {
                    tcp_service: tcp_service_inner,
                    tls_service: TlsConnector::service(tls),
                    timeout: handshake_timeout,
                };

                Some(actix_service::boxed::rc_service(tls_service))
            }

            #[cfg(feature = "rustls-0_23")]
            OurTlsConnector::Rustls023(tls) => {
                const H2: &[u8] = b"h2";

                use actix_tls::connect::rustls_0_23::{reexports::AsyncTlsStream, TlsConnector};

                #[allow(non_local_definitions)]
                impl<Io: ConnectionIo> IntoConnectionIo for TcpConnection<Uri, AsyncTlsStream<Io>> {
                    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                        let sock = self.into_parts().0;
                        let h2 = sock
                            .get_ref()
                            .1
                            .alpn_protocol()
                            .is_some_and(|protos| protos.windows(2).any(|w| w == H2));

                        if h2 {
                            (Box::new(sock), Protocol::Http2)
                        } else {
                            (Box::new(sock), Protocol::Http1)
                        }
                    }
                }

                let handshake_timeout = self.config.handshake_timeout;

                let tls_service = TlsConnectorService {
                    tcp_service: tcp_service_inner,
                    tls_service: TlsConnector::service(tls),
                    timeout: handshake_timeout,
                };

                Some(actix_service::boxed::rc_service(tls_service))
            }
        };

        let tcp_config = self.config.no_disconnect_timeout();

        let tcp_pool = ConnectionPool::new(tcp_service, tcp_config);

        let tls_config = self.config;
        let tls_pool =
            tls_service.map(move |tls_service| ConnectionPool::new(tls_service, tls_config));

        ConnectorServicePriv { tcp_pool, tls_pool }
    }
}

/// tcp service for map `TcpConnection<Uri, Io>` type to `(Io, Protocol)`
#[derive(Clone)]
pub struct TcpConnectorService<S: Clone> {
    service: S,
}

impl<S, Io> Service<Connect> for TcpConnectorService<S>
where
    S: Service<Connect, Response = TcpConnection<Uri, Io>, Error = ConnectError> + Clone + 'static,
{
    type Response = (Io, Protocol);
    type Error = ConnectError;
    type Future = TcpConnectorFuture<S::Future>;

    actix_service::forward_ready!(service);

    fn call(&self, req: Connect) -> Self::Future {
        TcpConnectorFuture {
            fut: self.service.call(req),
        }
    }
}

pin_project! {
    #[project = TcpConnectorFutureProj]
    pub struct TcpConnectorFuture<Fut> {
        #[pin]
        fut: Fut,
    }
}

impl<Fut, Io> Future for TcpConnectorFuture<Fut>
where
    Fut: Future<Output = Result<TcpConnection<Uri, Io>, ConnectError>>,
{
    type Output = Result<(Io, Protocol), ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project()
            .fut
            .poll(cx)
            .map_ok(|res| (res.into_parts().0, Protocol::Http1))
    }
}

/// service for establish tcp connection and do client tls handshake.
/// operation is canceled when timeout limit reached.
#[cfg(any(
    feature = "dangerous-h2c",
    feature = "openssl",
    feature = "rustls-0_20",
    feature = "rustls-0_21",
    feature = "rustls-0_22-webpki-roots",
    feature = "rustls-0_22-native-roots",
    feature = "rustls-0_23",
    feature = "rustls-0_23-webpki-roots",
    feature = "rustls-0_23-native-roots"
))]
struct TlsConnectorService<Tcp, Tls> {
    /// TCP connection is canceled on `TcpConnectorInnerService`'s timeout setting.
    tcp_service: Tcp,

    /// TLS connection is canceled on `TlsConnectorService`'s timeout setting.
    tls_service: Tls,

    timeout: Duration,
}

#[cfg(any(
    feature = "dangerous-h2c",
    feature = "openssl",
    feature = "rustls-0_20",
    feature = "rustls-0_21",
    feature = "rustls-0_22-webpki-roots",
    feature = "rustls-0_22-native-roots",
    feature = "rustls-0_23",
))]
impl<Tcp, Tls, IO> Service<Connect> for TlsConnectorService<Tcp, Tls>
where
    Tcp:
        Service<Connect, Response = TcpConnection<Uri, IO>, Error = ConnectError> + Clone + 'static,
    Tls: Service<TcpConnection<Uri, IO>, Error = std::io::Error> + Clone + 'static,
    Tls::Response: IntoConnectionIo,
    IO: ConnectionIo,
{
    type Response = (Box<dyn ConnectionIo>, Protocol);
    type Error = ConnectError;
    type Future = TlsConnectorFuture<Tls, Tcp::Future, Tls::Future>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.tcp_service.poll_ready(cx))?;
        ready!(self.tls_service.poll_ready(cx))?;
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: Connect) -> Self::Future {
        let fut = self.tcp_service.call(req);
        let tls_service = self.tls_service.clone();
        let timeout = self.timeout;

        TlsConnectorFuture::TcpConnect {
            fut,
            tls_service: Some(tls_service),
            timeout,
        }
    }
}

pin_project! {
    #[project = TlsConnectorProj]
    #[allow(clippy::large_enum_variant)]
    enum TlsConnectorFuture<S, Fut1, Fut2> {
        TcpConnect {
            #[pin]
            fut: Fut1,
            tls_service: Option<S>,
            timeout: Duration,
        },
        TlsConnect {
            #[pin]
            fut: Fut2,
            #[pin]
            timeout: Sleep,
        },
    }

}
/// helper trait for generic over different TlsStream types between tls crates.
trait IntoConnectionIo {
    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol);
}

impl<S, Io, Fut1, Fut2, Res> Future for TlsConnectorFuture<S, Fut1, Fut2>
where
    S: Service<TcpConnection<Uri, Io>, Response = Res, Error = std::io::Error, Future = Fut2>,
    S::Response: IntoConnectionIo,
    Fut1: Future<Output = Result<TcpConnection<Uri, Io>, ConnectError>>,
    Fut2: Future<Output = Result<S::Response, S::Error>>,
    Io: ConnectionIo,
{
    type Output = Result<(Box<dyn ConnectionIo>, Protocol), ConnectError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            TlsConnectorProj::TcpConnect {
                fut,
                tls_service,
                timeout,
            } => {
                let res = ready!(fut.poll(cx))?;
                let fut = tls_service
                    .take()
                    .expect("TlsConnectorFuture polled after complete")
                    .call(res);
                let timeout = sleep(*timeout);
                self.set(TlsConnectorFuture::TlsConnect { fut, timeout });
                self.poll(cx)
            }
            TlsConnectorProj::TlsConnect { fut, timeout } => match fut.poll(cx)? {
                Poll::Ready(res) => Poll::Ready(Ok(res.into_connection_io())),
                Poll::Pending => timeout.poll(cx).map(|_| Err(ConnectError::Timeout)),
            },
        }
    }
}

/// service for establish tcp connection.
/// operation is canceled when timeout limit reached.
#[derive(Clone)]
pub struct TcpConnectorInnerService<S: Clone> {
    service: S,
    timeout: Duration,
    local_address: Option<std::net::IpAddr>,
}

impl<S: Clone> TcpConnectorInnerService<S> {
    fn new(service: S, timeout: Duration, local_address: Option<std::net::IpAddr>) -> Self {
        Self {
            service,
            timeout,
            local_address,
        }
    }
}

impl<S, Io> Service<Connect> for TcpConnectorInnerService<S>
where
    S: Service<ConnectInfo<Uri>, Response = TcpConnection<Uri, Io>, Error = TcpConnectError>
        + Clone
        + 'static,
{
    type Response = S::Response;
    type Error = ConnectError;
    type Future = TcpConnectorInnerFuture<S::Future>;

    actix_service::forward_ready!(service);

    fn call(&self, req: Connect) -> Self::Future {
        let mut req = ConnectInfo::new(req.uri).set_addr(req.addr);

        if let Some(local_addr) = self.local_address {
            req = req.set_local_addr(local_addr);
        }

        TcpConnectorInnerFuture {
            fut: self.service.call(req),
            timeout: sleep(self.timeout),
        }
    }
}

pin_project! {
    #[project = TcpConnectorInnerFutureProj]
    pub struct TcpConnectorInnerFuture<Fut> {
        #[pin]
        fut: Fut,
        #[pin]
        timeout: Sleep,
    }
}

impl<Fut, Io> Future for TcpConnectorInnerFuture<Fut>
where
    Fut: Future<Output = Result<TcpConnection<Uri, Io>, TcpConnectError>>,
{
    type Output = Result<TcpConnection<Uri, Io>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.fut.poll(cx) {
            Poll::Ready(res) => Poll::Ready(res.map_err(ConnectError::from)),
            Poll::Pending => this.timeout.poll(cx).map(|_| Err(ConnectError::Timeout)),
        }
    }
}

/// Connector service for pooled Plain/Tls Tcp connections.
pub type ConnectorService<Svc, IO> = ConnectorServicePriv<
    TcpConnectorService<TcpConnectorInnerService<Svc>>,
    Rc<
        dyn Service<
            Connect,
            Response = (Box<dyn ConnectionIo>, Protocol),
            Error = ConnectError,
            Future = LocalBoxFuture<
                'static,
                Result<(Box<dyn ConnectionIo>, Protocol), ConnectError>,
            >,
        >,
    >,
    IO,
    Box<dyn ConnectionIo>,
>;

pub struct ConnectorServicePriv<S1, S2, Io1, Io2>
where
    S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError>,
    S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError>,
    Io1: ConnectionIo,
    Io2: ConnectionIo,
{
    tcp_pool: ConnectionPool<S1, Io1>,
    tls_pool: Option<ConnectionPool<S2, Io2>>,
}

impl<S1, S2, Io1, Io2> Service<Connect> for ConnectorServicePriv<S1, S2, Io1, Io2>
where
    S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError> + Clone + 'static,
    S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError> + Clone + 'static,
    Io1: ConnectionIo,
    Io2: ConnectionIo,
{
    type Response = Connection<Io1, Io2>;
    type Error = ConnectError;
    type Future = ConnectorServiceFuture<S1, S2, Io1, Io2>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.tcp_pool.poll_ready(cx))?;
        if let Some(ref tls_pool) = self.tls_pool {
            ready!(tls_pool.poll_ready(cx))?;
        }
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: Connect) -> Self::Future {
        match req.uri.scheme_str() {
            Some("https") | Some("wss") => match self.tls_pool {
                None => ConnectorServiceFuture::SslIsNotSupported,
                Some(ref pool) => ConnectorServiceFuture::Tls {
                    fut: pool.call(req),
                },
            },
            _ => ConnectorServiceFuture::Tcp {
                fut: self.tcp_pool.call(req),
            },
        }
    }
}

pin_project! {
    #[project = ConnectorServiceFutureProj]
    pub enum ConnectorServiceFuture<S1, S2, Io1, Io2>
    where
        S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError>,
        S1: Clone,
        S1: 'static,
        S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError>,
        S2: Clone,
        S2: 'static,
        Io1: ConnectionIo,
        Io2: ConnectionIo,
    {
        Tcp {
            #[pin]
            fut: <ConnectionPool<S1, Io1> as Service<Connect>>::Future
        },
        Tls {
            #[pin]
            fut:  <ConnectionPool<S2, Io2> as Service<Connect>>::Future
        },
        SslIsNotSupported
    }
}

impl<S1, S2, Io1, Io2> Future for ConnectorServiceFuture<S1, S2, Io1, Io2>
where
    S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError> + Clone + 'static,
    S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError> + Clone + 'static,
    Io1: ConnectionIo,
    Io2: ConnectionIo,
{
    type Output = Result<Connection<Io1, Io2>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            ConnectorServiceFutureProj::Tcp { fut } => fut.poll(cx).map_ok(Connection::Tcp),
            ConnectorServiceFutureProj::Tls { fut } => fut.poll(cx).map_ok(Connection::Tls),
            ConnectorServiceFutureProj::SslIsNotSupported => {
                Poll::Ready(Err(ConnectError::SslIsNotSupported))
            }
        }
    }
}

#[cfg(not(feature = "trust-dns"))]
mod resolver {
    use super::*;

    pub(super) fn resolver() -> Resolver {
        Resolver::default()
    }
}

#[cfg(feature = "trust-dns")]
mod resolver {
    use std::{cell::RefCell, net::SocketAddr};

    use actix_tls::connect::Resolve;
    use trust_dns_resolver::{
        config::{ResolverConfig, ResolverOpts},
        system_conf::read_system_conf,
        TokioAsyncResolver,
    };

    use super::*;

    pub(super) fn resolver() -> Resolver {
        // new type for impl Resolve trait for TokioAsyncResolver.
        struct TrustDnsResolver(TokioAsyncResolver);

        impl Resolve for TrustDnsResolver {
            fn lookup<'a>(
                &'a self,
                host: &'a str,
                port: u16,
            ) -> LocalBoxFuture<'a, Result<Vec<SocketAddr>, Box<dyn std::error::Error>>>
            {
                Box::pin(async move {
                    let res = self
                        .0
                        .lookup_ip(host)
                        .await?
                        .iter()
                        .map(|ip| SocketAddr::new(ip, port))
                        .collect();
                    Ok(res)
                })
            }
        }

        // resolver struct is cached in thread local so new clients can reuse the existing instance
        thread_local! {
            static TRUST_DNS_RESOLVER: RefCell<Option<Resolver>> = const { RefCell::new(None) };
        }

        // get from thread local or construct a new trust-dns resolver.
        TRUST_DNS_RESOLVER.with(|local| {
            let resolver = local.borrow().as_ref().map(Clone::clone);

            match resolver {
                Some(resolver) => resolver,

                None => {
                    let (cfg, opts) = match read_system_conf() {
                        Ok((cfg, opts)) => (cfg, opts),
                        Err(err) => {
                            log::error!("Trust-DNS can not load system config: {err}");
                            (ResolverConfig::default(), ResolverOpts::default())
                        }
                    };

                    let resolver = TokioAsyncResolver::tokio(cfg, opts);

                    // box trust dns resolver and put it in thread local.
                    let resolver = Resolver::custom(TrustDnsResolver(resolver));
                    *local.borrow_mut() = Some(resolver.clone());

                    resolver
                }
            }
        })
    }
}

#[cfg(feature = "dangerous-h2c")]
#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use actix_http::{HttpService, Request, Response, Version};
    use actix_http_test::test_server;
    use actix_service::ServiceFactoryExt as _;

    use super::*;
    use crate::Client;

    #[actix_rt::test]
    async fn h2c_connector() {
        let mut srv = test_server(|| {
            HttpService::build()
                .h2(|_req: Request| async { Ok::<_, Infallible>(Response::ok()) })
                .tcp()
                .map_err(|_| ())
        })
        .await;

        let connector = Connector {
            connector: TcpConnector::new(resolver::resolver()).service(),
            config: ConnectorConfig::default(),
            tls: OurTlsConnector::None,
        };

        let client = Client::builder().connector(connector).finish();

        let request = client.get(srv.surl("/")).send();
        let response = request.await.unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.version(), Version::HTTP_2);

        srv.stop().await;
    }
}

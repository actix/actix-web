use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_rt::net::TcpStream;
use actix_service::{apply_fn, Service, ServiceExt};
use actix_tls::connect::{
    new_connector, Connect as TcpConnect, Connection as TcpConnection, Resolver,
};
use actix_utils::timeout::{TimeoutError, TimeoutService};
use http::Uri;

use super::config::ConnectorConfig;
use super::connection::Connection;
use super::error::ConnectError;
use super::pool::{ConnectionPool, Protocol};
use super::Connect;

#[cfg(feature = "openssl")]
use actix_tls::connect::ssl::openssl::SslConnector as OpensslConnector;
#[cfg(feature = "rustls")]
use actix_tls::connect::ssl::rustls::ClientConfig;
#[cfg(feature = "rustls")]
use std::sync::Arc;

#[cfg(any(feature = "openssl", feature = "rustls"))]
enum SslConnector {
    #[cfg(feature = "openssl")]
    Openssl(OpensslConnector),
    #[cfg(feature = "rustls")]
    Rustls(Arc<ClientConfig>),
}
#[cfg(not(any(feature = "openssl", feature = "rustls")))]
type SslConnector = ();

/// Manages HTTP client network connectivity.
///
/// The `Connector` type uses a builder-like combinator pattern for service
/// construction that finishes by calling the `.finish()` method.
///
/// ```rust,ignore
/// use std::time::Duration;
/// use actix_http::client::Connector;
///
/// let connector = Connector::new()
///      .timeout(Duration::from_secs(5))
///      .finish();
/// ```
pub struct Connector<T, U> {
    connector: T,
    config: ConnectorConfig,
    #[allow(dead_code)]
    ssl: SslConnector,
    _phantom: PhantomData<U>,
}

trait Io: AsyncRead + AsyncWrite + Unpin {}
impl<T: AsyncRead + AsyncWrite + Unpin> Io for T {}

impl Connector<(), ()> {
    #[allow(clippy::new_ret_no_self, clippy::let_unit_value)]
    pub fn new() -> Connector<
        impl Service<
                TcpConnect<Uri>,
                Response = TcpConnection<Uri, TcpStream>,
                Error = actix_tls::connect::ConnectError,
            > + Clone,
        TcpStream,
    > {
        Connector {
            ssl: Self::build_ssl(vec![b"h2".to_vec(), b"http/1.1".to_vec()]),
            connector: new_connector(resolver::resolver()),
            config: ConnectorConfig::default(),
            _phantom: PhantomData,
        }
    }

    // Build Ssl connector with openssl, based on supplied alpn protocols
    #[cfg(feature = "openssl")]
    fn build_ssl(protocols: Vec<Vec<u8>>) -> SslConnector {
        use actix_tls::connect::ssl::openssl::SslMethod;
        use bytes::{BufMut, BytesMut};

        let mut alpn = BytesMut::with_capacity(20);
        for proto in protocols.iter() {
            alpn.put_u8(proto.len() as u8);
            alpn.put(proto.as_slice());
        }

        let mut ssl = OpensslConnector::builder(SslMethod::tls()).unwrap();
        let _ = ssl
            .set_alpn_protos(&alpn)
            .map_err(|e| error!("Can not set alpn protocol: {:?}", e));
        SslConnector::Openssl(ssl.build())
    }

    // Build Ssl connector with rustls, based on supplied alpn protocols
    #[cfg(all(not(feature = "openssl"), feature = "rustls"))]
    fn build_ssl(protocols: Vec<Vec<u8>>) -> SslConnector {
        let mut config = ClientConfig::new();
        config.set_protocols(&protocols);
        config.root_store.add_server_trust_anchors(
            &actix_tls::connect::ssl::rustls::TLS_SERVER_ROOTS,
        );
        SslConnector::Rustls(Arc::new(config))
    }

    // ssl turned off, provides empty ssl connector
    #[cfg(not(any(feature = "openssl", feature = "rustls")))]
    fn build_ssl(_: Vec<Vec<u8>>) -> SslConnector {}
}

impl<T, U> Connector<T, U> {
    /// Use custom connector.
    pub fn connector<T1, U1>(self, connector: T1) -> Connector<T1, U1>
    where
        U1: AsyncRead + AsyncWrite + Unpin + fmt::Debug,
        T1: Service<
                TcpConnect<Uri>,
                Response = TcpConnection<Uri, U1>,
                Error = actix_tls::connect::ConnectError,
            > + Clone,
    {
        Connector {
            connector,
            config: self.config,
            ssl: self.ssl,
            _phantom: PhantomData,
        }
    }
}

impl<T, U> Connector<T, U>
where
    U: AsyncRead + AsyncWrite + Unpin + fmt::Debug + 'static,
    T: Service<
            TcpConnect<Uri>,
            Response = TcpConnection<Uri, U>,
            Error = actix_tls::connect::ConnectError,
        > + Clone
        + 'static,
{
    /// Connection timeout, i.e. max time to connect to remote host including dns name resolution.
    /// Set to 1 second by default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    #[cfg(feature = "openssl")]
    /// Use custom `SslConnector` instance.
    pub fn ssl(mut self, connector: OpensslConnector) -> Self {
        self.ssl = SslConnector::Openssl(connector);
        self
    }

    #[cfg(feature = "rustls")]
    pub fn rustls(mut self, connector: Arc<ClientConfig>) -> Self {
        self.ssl = SslConnector::Rustls(connector);
        self
    }

    /// Maximum supported HTTP major version.
    ///
    /// Supported versions are HTTP/1.1 and HTTP/2.
    pub fn max_http_version(mut self, val: http::Version) -> Self {
        let versions = match val {
            http::Version::HTTP_11 => vec![b"http/1.1".to_vec()],
            http::Version::HTTP_2 => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            _ => {
                unimplemented!("actix-http:client: supported versions http/1.1, http/2")
            }
        };
        self.ssl = Connector::build_ssl(versions);
        self
    }

    /// Indicates the initial window size (in octets) for
    /// HTTP2 stream-level flow control for received data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_window_size(mut self, size: u32) -> Self {
        self.config.stream_window_size = size;
        self
    }

    /// Indicates the initial window size (in octets) for
    /// HTTP2 connection-level flow control for received data.
    ///
    /// The default value is 65,535 and is good for APIs, but not for big objects.
    pub fn initial_connection_window_size(mut self, size: u32) -> Self {
        self.config.conn_window_size = size;
        self
    }

    /// Set total number of simultaneous connections per type of scheme.
    ///
    /// If limit is 0, the connector has no limit.
    /// The default limit size is 100.
    pub fn limit(mut self, limit: usize) -> Self {
        self.config.limit = limit;
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

    /// Finish configuration process and create connector service.
    /// The Connector builder always concludes by calling `finish()` last in
    /// its combinator chain.
    pub fn finish(
        self,
    ) -> impl Service<Connect, Response = impl Connection, Error = ConnectError> + Clone
    {
        #[cfg(not(any(feature = "openssl", feature = "rustls")))]
        {
            let connector = TimeoutService::new(
                self.config.timeout,
                apply_fn(self.connector, |msg: Connect, srv| {
                    srv.call(TcpConnect::new(msg.uri).set_addr(msg.addr))
                })
                .map_err(ConnectError::from)
                .map(|stream| (stream.into_parts().0, Protocol::Http1)),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
            });

            connect_impl::InnerConnector {
                tcp_pool: ConnectionPool::new(
                    connector,
                    self.config.no_disconnect_timeout(),
                ),
            }
        }
        #[cfg(any(feature = "openssl", feature = "rustls"))]
        {
            const H2: &[u8] = b"h2";
            use actix_service::{boxed::service, pipeline};
            #[cfg(feature = "openssl")]
            use actix_tls::connect::ssl::openssl::OpensslConnector;
            #[cfg(feature = "rustls")]
            use actix_tls::connect::ssl::rustls::{RustlsConnector, Session};

            let ssl_service = TimeoutService::new(
                self.config.timeout,
                pipeline(
                    apply_fn(self.connector.clone(), |msg: Connect, srv| {
                        srv.call(TcpConnect::new(msg.uri).set_addr(msg.addr))
                    })
                    .map_err(ConnectError::from),
                )
                .and_then(match self.ssl {
                    #[cfg(feature = "openssl")]
                    SslConnector::Openssl(ssl) => service(
                        OpensslConnector::service(ssl)
                            .map(|stream| {
                                let sock = stream.into_parts().0;
                                let h2 = sock
                                    .ssl()
                                    .selected_alpn_protocol()
                                    .map(|protos| protos.windows(2).any(|w| w == H2))
                                    .unwrap_or(false);
                                if h2 {
                                    (Box::new(sock) as Box<dyn Io>, Protocol::Http2)
                                } else {
                                    (Box::new(sock) as Box<dyn Io>, Protocol::Http1)
                                }
                            })
                            .map_err(ConnectError::from),
                    ),
                    #[cfg(feature = "rustls")]
                    SslConnector::Rustls(ssl) => service(
                        RustlsConnector::service(ssl)
                            .map_err(ConnectError::from)
                            .map(|stream| {
                                let sock = stream.into_parts().0;
                                let h2 = sock
                                    .get_ref()
                                    .1
                                    .get_alpn_protocol()
                                    .map(|protos| protos.windows(2).any(|w| w == H2))
                                    .unwrap_or(false);
                                if h2 {
                                    (Box::new(sock) as Box<dyn Io>, Protocol::Http2)
                                } else {
                                    (Box::new(sock) as Box<dyn Io>, Protocol::Http1)
                                }
                            }),
                    ),
                }),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
            });

            let tcp_service = TimeoutService::new(
                self.config.timeout,
                apply_fn(self.connector, |msg: Connect, srv| {
                    srv.call(TcpConnect::new(msg.uri).set_addr(msg.addr))
                })
                .map_err(ConnectError::from)
                .map(|stream| (stream.into_parts().0, Protocol::Http1)),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
            });

            connect_impl::InnerConnector {
                tcp_pool: ConnectionPool::new(
                    tcp_service,
                    self.config.no_disconnect_timeout(),
                ),
                ssl_pool: ConnectionPool::new(ssl_service, self.config),
            }
        }
    }
}

#[cfg(not(any(feature = "openssl", feature = "rustls")))]
mod connect_impl {
    use std::task::{Context, Poll};

    use futures_core::future::LocalBoxFuture;

    use super::*;
    use crate::client::connection::IoConnection;

    pub(crate) struct InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin + 'static,
        T: Service<Connect, Response = (Io, Protocol), Error = ConnectError> + 'static,
    {
        pub(crate) tcp_pool: ConnectionPool<T, Io>,
    }

    impl<T, Io> Clone for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin + 'static,
        T: Service<Connect, Response = (Io, Protocol), Error = ConnectError> + 'static,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
            }
        }
    }

    impl<T, Io> Service<Connect> for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + Unpin + 'static,
        T: Service<Connect, Response = (Io, Protocol), Error = ConnectError> + 'static,
    {
        type Response = IoConnection<Io>;
        type Error = ConnectError;
        type Future = LocalBoxFuture<'static, Result<IoConnection<Io>, ConnectError>>;

        fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.tcp_pool.poll_ready(cx)
        }

        fn call(&self, req: Connect) -> Self::Future {
            match req.uri.scheme_str() {
                Some("https") | Some("wss") => {
                    Box::pin(async { Err(ConnectError::SslIsNotSupported) })
                }
                _ => self.tcp_pool.call(req),
            }
        }
    }
}

#[cfg(any(feature = "openssl", feature = "rustls"))]
mod connect_impl {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::*;
    use crate::client::connection::EitherIoConnection;

    pub(crate) struct InnerConnector<S1, S2, Io1, Io2>
    where
        S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError> + 'static,
        S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError> + 'static,
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        pub(crate) tcp_pool: ConnectionPool<S1, Io1>,
        pub(crate) ssl_pool: ConnectionPool<S2, Io2>,
    }

    impl<S1, S2, Io1, Io2> Clone for InnerConnector<S1, S2, Io1, Io2>
    where
        S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError> + 'static,
        S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError> + 'static,
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
                ssl_pool: self.ssl_pool.clone(),
            }
        }
    }

    impl<S1, S2, Io1, Io2> Service<Connect> for InnerConnector<S1, S2, Io1, Io2>
    where
        S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError> + 'static,
        S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError> + 'static,
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        type Response = EitherIoConnection<Io1, Io2>;
        type Error = ConnectError;
        type Future = InnerConnectorResponse<S1, S2, Io1, Io2>;

        fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.tcp_pool.poll_ready(cx)
        }

        fn call(&self, req: Connect) -> Self::Future {
            match req.uri.scheme_str() {
                Some("https") | Some("wss") => {
                    InnerConnectorResponse::Io2(self.ssl_pool.call(req))
                }
                _ => InnerConnectorResponse::Io1(self.tcp_pool.call(req)),
            }
        }
    }

    #[pin_project::pin_project(project = InnerConnectorProj)]
    pub(crate) enum InnerConnectorResponse<S1, S2, Io1, Io2>
    where
        S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError> + 'static,
        S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError> + 'static,
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        Io1(#[pin] <ConnectionPool<S1, Io1> as Service<Connect>>::Future),
        Io2(#[pin] <ConnectionPool<S2, Io2> as Service<Connect>>::Future),
    }

    impl<S1, S2, Io1, Io2> Future for InnerConnectorResponse<S1, S2, Io1, Io2>
    where
        S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError> + 'static,
        S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError> + 'static,
        Io1: AsyncRead + AsyncWrite + Unpin + 'static,
        Io2: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        type Output = Result<EitherIoConnection<Io1, Io2>, ConnectError>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            match self.project() {
                InnerConnectorProj::Io1(fut) => {
                    fut.poll(cx).map_ok(EitherIoConnection::A)
                }
                InnerConnectorProj::Io2(fut) => {
                    fut.poll(cx).map_ok(EitherIoConnection::B)
                }
            }
        }
    }
}

#[cfg(not(feature = "trust-dns"))]
mod resolver {
    use super::*;

    pub(super) fn resolver() -> Resolver {
        Resolver::Default
    }
}

#[cfg(feature = "trust-dns")]
mod resolver {
    use std::{cell::RefCell, net::SocketAddr};

    use actix_tls::connect::Resolve;
    use futures_core::future::LocalBoxFuture;
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

        // dns struct is cached in thread local.
        // so new client constructor can reuse the existing dns resolver.
        thread_local! {
            static TRUST_DNS_RESOLVER: RefCell<Option<Resolver>> = RefCell::new(None);
        }

        // get from thread local or construct a new trust-dns resolver.
        TRUST_DNS_RESOLVER.with(|local| {
            let resolver = local.borrow().as_ref().map(Clone::clone);
            match resolver {
                Some(resolver) => resolver,
                None => {
                    let (cfg, opts) = match read_system_conf() {
                        Ok((cfg, opts)) => (cfg, opts),
                        Err(e) => {
                            log::error!("TRust-DNS can not load system config: {}", e);
                            (ResolverConfig::default(), ResolverOpts::default())
                        }
                    };

                    let resolver = TokioAsyncResolver::tokio(cfg, opts).unwrap();

                    // box trust dns resolver and put it in thread local.
                    let resolver = Resolver::new_custom(TrustDnsResolver(resolver));
                    *local.borrow_mut() = Some(resolver.clone());
                    resolver
                }
            }
        })
    }
}

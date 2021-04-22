use std::{
    fmt,
    future::Future,
    net::IpAddr,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{
    net::{ActixStream, TcpStream},
    time::{sleep, Sleep},
};
use actix_service::Service;
use actix_tls::connect::{
    new_connector, Connect as TcpConnect, ConnectError as TcpConnectError,
    Connection as TcpConnection, Resolver,
};
use futures_core::{future::LocalBoxFuture, ready};
use http::Uri;
use pin_project::pin_project;

use super::config::ConnectorConfig;
use super::connection::{Connection, ConnectionIo};
use super::error::ConnectError;
use super::pool::ConnectionPool;
use super::Connect;
use super::Protocol;

#[cfg(feature = "openssl")]
use actix_tls::connect::ssl::openssl::SslConnector as OpensslConnector;
#[cfg(feature = "rustls")]
use actix_tls::connect::ssl::rustls::ClientConfig;

enum SslConnector {
    #[allow(dead_code)]
    None,
    #[cfg(feature = "openssl")]
    Openssl(OpensslConnector),
    #[cfg(feature = "rustls")]
    Rustls(std::sync::Arc<ClientConfig>),
}

/// Manages HTTP client network connectivity.
///
/// The `Connector` type uses a builder-like combinator pattern for service
/// construction that finishes by calling the `.finish()` method.
///
/// ```ignore
/// use std::time::Duration;
/// use actix_http::client::Connector;
///
/// let connector = Connector::new()
///      .timeout(Duration::from_secs(5))
///      .finish();
/// ```
pub struct Connector<T> {
    connector: T,
    config: ConnectorConfig,
    #[allow(dead_code)]
    ssl: SslConnector,
}

impl Connector<()> {
    #[allow(clippy::new_ret_no_self, clippy::let_unit_value)]
    pub fn new() -> Connector<
        impl Service<
                TcpConnect<Uri>,
                Response = TcpConnection<Uri, TcpStream>,
                Error = actix_tls::connect::ConnectError,
            > + Clone,
    > {
        Connector {
            ssl: Self::build_ssl(vec![b"h2".to_vec(), b"http/1.1".to_vec()]),
            connector: new_connector(resolver::resolver()),
            config: ConnectorConfig::default(),
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
        SslConnector::Rustls(std::sync::Arc::new(config))
    }

    // ssl turned off, provides empty ssl connector
    #[cfg(not(any(feature = "openssl", feature = "rustls")))]
    fn build_ssl(_: Vec<Vec<u8>>) -> SslConnector {
        SslConnector::None
    }
}

impl<S> Connector<S> {
    /// Use custom connector.
    pub fn connector<S1, Io1>(self, connector: S1) -> Connector<S1>
    where
        Io1: ActixStream + fmt::Debug + 'static,
        S1: Service<
                TcpConnect<Uri>,
                Response = TcpConnection<Uri, Io1>,
                Error = TcpConnectError,
            > + Clone,
    {
        Connector {
            connector,
            config: self.config,
            ssl: self.ssl,
        }
    }
}

impl<S, Io> Connector<S>
where
    // Note:
    // Input Io type is bound to ActixStream trait but internally in client module they
    // are bound to ConnectionIo trait alias. And latter is the trait exposed to public
    // in the form of Box<dyn ConnectionIo> type.
    //
    // This remap is to hide ActixStream's trait methods. They are not meant to be called
    // from user code.
    Io: ActixStream + fmt::Debug + 'static,
    S: Service<
            TcpConnect<Uri>,
            Response = TcpConnection<Uri, Io>,
            Error = TcpConnectError,
        > + Clone
        + 'static,
{
    /// Tcp connection timeout, i.e. max time to connect to remote host including dns name
    /// resolution. Set to 5 second by default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Tls handshake timeout, i.e. max time to do tls handshake with remote host after tcp
    /// connection established. Set to 5 second by default.
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.config.handshake_timeout = timeout;
        self
    }

    #[cfg(feature = "openssl")]
    /// Use custom `SslConnector` instance.
    pub fn ssl(mut self, connector: OpensslConnector) -> Self {
        self.ssl = SslConnector::Openssl(connector);
        self
    }

    #[cfg(feature = "rustls")]
    /// Use custom `SslConnector` instance.
    pub fn rustls(mut self, connector: std::sync::Arc<ClientConfig>) -> Self {
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

    /// Set local IP Address the connector would use for establishing connection.
    pub fn local_address(mut self, addr: IpAddr) -> Self {
        self.config.local_address = Some(addr);
        self
    }

    /// Finish configuration process and create connector service.
    /// The Connector builder always concludes by calling `finish()` last in
    /// its combinator chain.
    pub fn finish(self) -> ConnectorService<S, Io> {
        let local_address = self.config.local_address;
        let timeout = self.config.timeout;

        let tcp_service_inner =
            TcpConnectorInnerService::new(self.connector, timeout, local_address);

        #[allow(clippy::redundant_clone)]
        let tcp_service = TcpConnectorService {
            service: tcp_service_inner.clone(),
        };

        let tls_service = match self.ssl {
            SslConnector::None => None,
            #[cfg(feature = "openssl")]
            SslConnector::Openssl(tls) => {
                const H2: &[u8] = b"h2";

                use actix_tls::connect::ssl::openssl::{OpensslConnector, SslStream};

                impl<Io: ConnectionIo> IntoConnectionIo for TcpConnection<Uri, SslStream<Io>> {
                    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                        let sock = self.into_parts().0;
                        let h2 = sock
                            .ssl()
                            .selected_alpn_protocol()
                            .map(|protos| protos.windows(2).any(|w| w == H2))
                            .unwrap_or(false);
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
                    tls_service: OpensslConnector::service(tls),
                    timeout: handshake_timeout,
                };

                Some(actix_service::boxed::rc_service(tls_service))
            }
            #[cfg(feature = "rustls")]
            SslConnector::Rustls(tls) => {
                const H2: &[u8] = b"h2";

                use actix_tls::connect::ssl::rustls::{
                    RustlsConnector, Session, TlsStream,
                };

                impl<Io: ConnectionIo> IntoConnectionIo for TcpConnection<Uri, TlsStream<Io>> {
                    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol) {
                        let sock = self.into_parts().0;
                        let h2 = sock
                            .get_ref()
                            .1
                            .get_alpn_protocol()
                            .map(|protos| protos.windows(2).any(|w| w == H2))
                            .unwrap_or(false);
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
                    tls_service: RustlsConnector::service(tls),
                    timeout: handshake_timeout,
                };

                Some(actix_service::boxed::rc_service(tls_service))
            }
        };

        let tcp_config = self.config.no_disconnect_timeout();

        let tcp_pool = ConnectionPool::new(tcp_service, tcp_config);

        let tls_config = self.config;
        let tls_pool = tls_service
            .map(move |tls_service| ConnectionPool::new(tls_service, tls_config));

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
    S: Service<Connect, Response = TcpConnection<Uri, Io>, Error = ConnectError>
        + Clone
        + 'static,
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

#[pin_project]
pub struct TcpConnectorFuture<Fut> {
    #[pin]
    fut: Fut,
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
struct TlsConnectorService<S, St> {
    /// tcp connection is canceled on `TcpConnectorInnerService`'s timeout setting.
    tcp_service: S,
    /// tls connection is canceled on `TlsConnectorService`'s timeout setting.
    tls_service: St,
    timeout: Duration,
}

impl<S, St, Io> Service<Connect> for TlsConnectorService<S, St>
where
    S: Service<Connect, Response = TcpConnection<Uri, Io>, Error = ConnectError>
        + Clone
        + 'static,
    St: Service<TcpConnection<Uri, Io>, Error = std::io::Error> + Clone + 'static,
    Io: ConnectionIo,
    St::Response: IntoConnectionIo,
{
    type Response = (Box<dyn ConnectionIo>, Protocol);
    type Error = ConnectError;
    type Future = TlsConnectorFuture<St, S::Future, St::Future>;

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

#[pin_project(project = TlsConnectorProj)]
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

/// helper trait for generic over different TlsStream types between tls crates.
trait IntoConnectionIo {
    fn into_connection_io(self) -> (Box<dyn ConnectionIo>, Protocol);
}

impl<S, Io, Fut1, Fut2, Res> Future for TlsConnectorFuture<S, Fut1, Fut2>
where
    S: Service<
        TcpConnection<Uri, Io>,
        Response = Res,
        Error = std::io::Error,
        Future = Fut2,
    >,
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
    fn new(
        service: S,
        timeout: Duration,
        local_address: Option<std::net::IpAddr>,
    ) -> Self {
        Self {
            service,
            timeout,
            local_address,
        }
    }
}

impl<S, Io> Service<Connect> for TcpConnectorInnerService<S>
where
    S: Service<
            TcpConnect<Uri>,
            Response = TcpConnection<Uri, Io>,
            Error = TcpConnectError,
        > + Clone
        + 'static,
{
    type Response = S::Response;
    type Error = ConnectError;
    type Future = TcpConnectorInnerFuture<S::Future>;

    actix_service::forward_ready!(service);

    fn call(&self, req: Connect) -> Self::Future {
        let mut req = TcpConnect::new(req.uri).set_addr(req.addr);

        if let Some(local_addr) = self.local_address {
            req = req.set_local_addr(local_addr);
        }

        TcpConnectorInnerFuture {
            fut: self.service.call(req),
            timeout: sleep(self.timeout),
        }
    }
}

#[pin_project]
pub struct TcpConnectorInnerFuture<Fut> {
    #[pin]
    fut: Fut,
    #[pin]
    timeout: Sleep,
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
pub type ConnectorService<S, Io> = ConnectorServicePriv<
    TcpConnectorService<TcpConnectorInnerService<S>>,
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
    Io,
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
    S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError>
        + Clone
        + 'static,
    S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError>
        + Clone
        + 'static,
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
                Some(ref pool) => ConnectorServiceFuture::Tls(pool.call(req)),
            },
            _ => ConnectorServiceFuture::Tcp(self.tcp_pool.call(req)),
        }
    }
}

#[pin_project(project = ConnectorServiceProj)]
pub enum ConnectorServiceFuture<S1, S2, Io1, Io2>
where
    S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError>
        + Clone
        + 'static,
    S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError>
        + Clone
        + 'static,
    Io1: ConnectionIo,
    Io2: ConnectionIo,
{
    Tcp(#[pin] <ConnectionPool<S1, Io1> as Service<Connect>>::Future),
    Tls(#[pin] <ConnectionPool<S2, Io2> as Service<Connect>>::Future),
    SslIsNotSupported,
}

impl<S1, S2, Io1, Io2> Future for ConnectorServiceFuture<S1, S2, Io1, Io2>
where
    S1: Service<Connect, Response = (Io1, Protocol), Error = ConnectError>
        + Clone
        + 'static,
    S2: Service<Connect, Response = (Io2, Protocol), Error = ConnectError>
        + Clone
        + 'static,
    Io1: ConnectionIo,
    Io2: ConnectionIo,
{
    type Output = Result<Connection<Io1, Io2>, ConnectError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            ConnectorServiceProj::Tcp(fut) => fut.poll(cx).map_ok(Connection::Tcp),
            ConnectorServiceProj::Tls(fut) => fut.poll(cx).map_ok(Connection::Tls),
            ConnectorServiceProj::SslIsNotSupported => {
                Poll::Ready(Err(ConnectError::SslIsNotSupported))
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

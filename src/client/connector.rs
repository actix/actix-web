use std::time::Duration;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_connector::{Resolver, TcpConnector};
use actix_service::{Service, ServiceExt};
use actix_utils::timeout::{TimeoutError, TimeoutService};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};

use super::connect::Connect;
use super::connection::Connection;
use super::error::ConnectorError;
use super::pool::{ConnectionPool, Protocol};

#[cfg(feature = "ssl")]
use openssl::ssl::SslConnector;

#[cfg(not(feature = "ssl"))]
type SslConnector = ();

/// Http client connector builde instance.
/// `Connector` type uses builder-like pattern for connector service construction.
pub struct Connector {
    resolver: Resolver<Connect>,
    timeout: Duration,
    conn_lifetime: Duration,
    conn_keep_alive: Duration,
    disconnect_timeout: Duration,
    limit: usize,
    #[allow(dead_code)]
    connector: SslConnector,
}

impl Default for Connector {
    fn default() -> Connector {
        let connector = {
            #[cfg(feature = "ssl")]
            {
                use log::error;
                use openssl::ssl::{SslConnector, SslMethod};

                let mut ssl = SslConnector::builder(SslMethod::tls()).unwrap();
                let _ = ssl
                    .set_alpn_protos(b"\x02h2\x08http/1.1")
                    .map_err(|e| error!("Can not set alpn protocol: {:?}", e));
                ssl.build()
            }
            #[cfg(not(feature = "ssl"))]
            {}
        };

        Connector {
            connector,
            resolver: Resolver::default(),
            timeout: Duration::from_secs(1),
            conn_lifetime: Duration::from_secs(75),
            conn_keep_alive: Duration::from_secs(15),
            disconnect_timeout: Duration::from_millis(3000),
            limit: 100,
        }
    }
}

impl Connector {
    /// Use custom resolver.
    pub fn resolver(mut self, resolver: Resolver<Connect>) -> Self {
        self.resolver = resolver;;
        self
    }

    /// Use custom resolver configuration.
    pub fn resolver_config(mut self, cfg: ResolverConfig, opts: ResolverOpts) -> Self {
        self.resolver = Resolver::new(cfg, opts);
        self
    }

    /// Connection timeout, i.e. max time to connect to remote host including dns name resolution.
    /// Set to 1 second by default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[cfg(feature = "ssl")]
    /// Use custom `SslConnector` instance.
    pub fn ssl(mut self, connector: SslConnector) -> Self {
        self.connector = connector;
        self
    }

    /// Set total number of simultaneous connections per type of scheme.
    ///
    /// If limit is 0, the connector has no limit.
    /// The default limit size is 100.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set keep-alive period for opened connection.
    ///
    /// Keep-alive period is the period between connection usage. If
    /// the delay between repeated usages of the same connection
    /// exceeds this period, the connection is closed.
    /// Default keep-alive period is 15 seconds.
    pub fn conn_keep_alive(mut self, dur: Duration) -> Self {
        self.conn_keep_alive = dur;
        self
    }

    /// Set max lifetime period for connection.
    ///
    /// Connection lifetime is max lifetime of any opened connection
    /// until it is closed regardless of keep-alive period.
    /// Default lifetime period is 75 seconds.
    pub fn conn_lifetime(mut self, dur: Duration) -> Self {
        self.conn_lifetime = dur;
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
        self.disconnect_timeout = dur;
        self
    }

    /// Finish configuration process and create connector service.
    pub fn service(
        self,
    ) -> impl Service<Connect, Response = impl Connection, Error = ConnectorError> + Clone
    {
        #[cfg(not(feature = "ssl"))]
        {
            let connector = TimeoutService::new(
                self.timeout,
                self.resolver.map_err(ConnectorError::from).and_then(
                    TcpConnector::default()
                        .from_err()
                        .map(|(msg, io)| (msg, io, Protocol::Http1)),
                ),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectorError::Timeout,
            });

            connect_impl::InnerConnector {
                tcp_pool: ConnectionPool::new(
                    connector,
                    self.conn_lifetime,
                    self.conn_keep_alive,
                    None,
                    self.limit,
                ),
            }
        }
        #[cfg(feature = "ssl")]
        {
            const H2: &[u8] = b"h2";
            use actix_connector::ssl::OpensslConnector;

            let ssl_service = TimeoutService::new(
                self.timeout,
                self.resolver
                    .clone()
                    .map_err(ConnectorError::from)
                    .and_then(TcpConnector::default().from_err())
                    .and_then(
                        OpensslConnector::service(self.connector)
                            .map_err(ConnectorError::from)
                            .map(|(msg, io)| {
                                let h2 = io
                                    .get_ref()
                                    .ssl()
                                    .selected_alpn_protocol()
                                    .map(|protos| protos.windows(2).any(|w| w == H2))
                                    .unwrap_or(false);
                                if h2 {
                                    (msg, io, Protocol::Http2)
                                } else {
                                    (msg, io, Protocol::Http1)
                                }
                            }),
                    ),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectorError::Timeout,
            });

            let tcp_service = TimeoutService::new(
                self.timeout,
                self.resolver.map_err(ConnectorError::from).and_then(
                    TcpConnector::default()
                        .from_err()
                        .map(|(msg, io)| (msg, io, Protocol::Http1)),
                ),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectorError::Timeout,
            });

            connect_impl::InnerConnector {
                tcp_pool: ConnectionPool::new(
                    tcp_service,
                    self.conn_lifetime,
                    self.conn_keep_alive,
                    None,
                    self.limit,
                ),
                ssl_pool: ConnectionPool::new(
                    ssl_service,
                    self.conn_lifetime,
                    self.conn_keep_alive,
                    Some(self.disconnect_timeout),
                    self.limit,
                ),
            }
        }
    }
}

#[cfg(not(feature = "ssl"))]
mod connect_impl {
    use futures::future::{err, Either, FutureResult};
    use futures::Poll;

    use super::*;
    use crate::client::connection::IoConnection;

    pub(crate) struct InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io, Protocol), Error = ConnectorError>,
    {
        pub(crate) tcp_pool: ConnectionPool<T, Io>,
    }

    impl<T, Io> Clone for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io, Protocol), Error = ConnectorError>
            + Clone,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
            }
        }
    }

    impl<T, Io> Service<Connect> for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io, Protocol), Error = ConnectorError>,
    {
        type Response = IoConnection<Io>;
        type Error = ConnectorError;
        type Future = Either<
            <ConnectionPool<T, Io> as Service<Connect>>::Future,
            FutureResult<IoConnection<Io>, ConnectorError>,
        >;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            self.tcp_pool.poll_ready()
        }

        fn call(&mut self, req: Connect) -> Self::Future {
            if req.is_secure() {
                Either::B(err(ConnectorError::SslIsNotSupported))
            } else if let Err(e) = req.validate() {
                Either::B(err(e))
            } else {
                Either::A(self.tcp_pool.call(req))
            }
        }
    }
}

#[cfg(feature = "ssl")]
mod connect_impl {
    use std::marker::PhantomData;

    use futures::future::{err, Either, FutureResult};
    use futures::{Async, Future, Poll};

    use super::*;
    use crate::client::connection::EitherConnection;

    pub(crate) struct InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<
            Connect,
            Response = (Connect, Io1, Protocol),
            Error = ConnectorError,
        >,
        T2: Service<
            Connect,
            Response = (Connect, Io2, Protocol),
            Error = ConnectorError,
        >,
    {
        pub(crate) tcp_pool: ConnectionPool<T1, Io1>,
        pub(crate) ssl_pool: ConnectionPool<T2, Io2>,
    }

    impl<T1, T2, Io1, Io2> Clone for InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<
                Connect,
                Response = (Connect, Io1, Protocol),
                Error = ConnectorError,
            > + Clone,
        T2: Service<
                Connect,
                Response = (Connect, Io2, Protocol),
                Error = ConnectorError,
            > + Clone,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
                ssl_pool: self.ssl_pool.clone(),
            }
        }
    }

    impl<T1, T2, Io1, Io2> Service<Connect> for InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<
            Connect,
            Response = (Connect, Io1, Protocol),
            Error = ConnectorError,
        >,
        T2: Service<
            Connect,
            Response = (Connect, Io2, Protocol),
            Error = ConnectorError,
        >,
    {
        type Response = EitherConnection<Io1, Io2>;
        type Error = ConnectorError;
        type Future = Either<
            FutureResult<Self::Response, Self::Error>,
            Either<
                InnerConnectorResponseA<T1, Io1, Io2>,
                InnerConnectorResponseB<T2, Io1, Io2>,
            >,
        >;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            self.tcp_pool.poll_ready()
        }

        fn call(&mut self, req: Connect) -> Self::Future {
            if let Err(e) = req.validate() {
                Either::A(err(e))
            } else if req.is_secure() {
                Either::B(Either::B(InnerConnectorResponseB {
                    fut: self.ssl_pool.call(req),
                    _t: PhantomData,
                }))
            } else {
                Either::B(Either::A(InnerConnectorResponseA {
                    fut: self.tcp_pool.call(req),
                    _t: PhantomData,
                }))
            }
        }
    }

    pub(crate) struct InnerConnectorResponseA<T, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io1, Protocol), Error = ConnectorError>,
    {
        fut: <ConnectionPool<T, Io1> as Service<Connect>>::Future,
        _t: PhantomData<Io2>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseA<T, Io1, Io2>
    where
        T: Service<Connect, Response = (Connect, Io1, Protocol), Error = ConnectorError>,
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
    {
        type Item = EitherConnection<Io1, Io2>;
        type Error = ConnectorError;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            match self.fut.poll()? {
                Async::NotReady => Ok(Async::NotReady),
                Async::Ready(res) => Ok(Async::Ready(EitherConnection::A(res))),
            }
        }
    }

    pub(crate) struct InnerConnectorResponseB<T, Io1, Io2>
    where
        Io2: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io2, Protocol), Error = ConnectorError>,
    {
        fut: <ConnectionPool<T, Io2> as Service<Connect>>::Future,
        _t: PhantomData<Io1>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseB<T, Io1, Io2>
    where
        T: Service<Connect, Response = (Connect, Io2, Protocol), Error = ConnectorError>,
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
    {
        type Item = EitherConnection<Io1, Io2>;
        type Error = ConnectorError;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            match self.fut.poll()? {
                Async::NotReady => Ok(Async::NotReady),
                Async::Ready(res) => Ok(Async::Ready(EitherConnection::B(res))),
            }
        }
    }
}

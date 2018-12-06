use std::time::Duration;
use std::{fmt, io};

use actix_net::connector::TcpConnector;
use actix_net::resolver::Resolver;
use actix_net::service::{Service, ServiceExt};
use actix_net::timeout::{TimeoutError, TimeoutService};
use futures::future::Either;
use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};

use super::connect::Connect;
use super::connection::{Connection, IoConnection};
use super::error::ConnectorError;
use super::pool::ConnectionPool;

#[cfg(feature = "ssl")]
use actix_net::ssl::OpensslConnector;
#[cfg(feature = "ssl")]
use openssl::ssl::{SslConnector, SslMethod};

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
                SslConnector::builder(SslMethod::tls()).unwrap().build()
            }
            #[cfg(not(feature = "ssl"))]
            {
                ()
            }
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
                self.resolver
                    .map_err(ConnectorError::from)
                    .and_then(TcpConnector::default().from_err()),
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
            let ssl_service = TimeoutService::new(
                self.timeout,
                self.resolver
                    .clone()
                    .map_err(ConnectorError::from)
                    .and_then(TcpConnector::default().from_err())
                    .and_then(
                        OpensslConnector::service(self.connector)
                            .map_err(ConnectorError::SslError),
                    ),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectorError::Timeout,
            });

            let tcp_service = TimeoutService::new(
                self.timeout,
                self.resolver
                    .map_err(ConnectorError::from)
                    .and_then(TcpConnector::default().from_err()),
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
    use super::*;
    use futures::future::{err, FutureResult};

    pub(crate) struct InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io), Error = ConnectorError>,
    {
        pub(crate) tcp_pool: ConnectionPool<T, Io>,
    }

    impl<T, Io> Clone for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io), Error = ConnectorError> + Clone,
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
        T: Service<Connect, Response = (Connect, Io), Error = ConnectorError>,
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

    use futures::future::{err, FutureResult};
    use futures::{Async, Future, Poll};

    use super::*;

    pub(crate) struct InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<Connect, Response = (Connect, Io1), Error = ConnectorError>,
        T2: Service<Connect, Response = (Connect, Io2), Error = ConnectorError>,
    {
        pub(crate) tcp_pool: ConnectionPool<T1, Io1>,
        pub(crate) ssl_pool: ConnectionPool<T2, Io2>,
    }

    impl<T1, T2, Io1, Io2> Clone for InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<Connect, Response = (Connect, Io1), Error = ConnectorError> + Clone,
        T2: Service<Connect, Response = (Connect, Io2), Error = ConnectorError> + Clone,
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
        T1: Service<Connect, Response = (Connect, Io1), Error = ConnectorError>,
        T2: Service<Connect, Response = (Connect, Io2), Error = ConnectorError>,
    {
        type Response = IoEither<IoConnection<Io1>, IoConnection<Io2>>;
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
        T: Service<Connect, Response = (Connect, Io1), Error = ConnectorError>,
    {
        fut: <ConnectionPool<T, Io1> as Service<Connect>>::Future,
        _t: PhantomData<Io2>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseA<T, Io1, Io2>
    where
        T: Service<Connect, Response = (Connect, Io1), Error = ConnectorError>,
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
    {
        type Item = IoEither<IoConnection<Io1>, IoConnection<Io2>>;
        type Error = ConnectorError;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            match self.fut.poll()? {
                Async::NotReady => Ok(Async::NotReady),
                Async::Ready(res) => Ok(Async::Ready(IoEither::A(res))),
            }
        }
    }

    pub(crate) struct InnerConnectorResponseB<T, Io1, Io2>
    where
        Io2: AsyncRead + AsyncWrite + 'static,
        T: Service<Connect, Response = (Connect, Io2), Error = ConnectorError>,
    {
        fut: <ConnectionPool<T, Io2> as Service<Connect>>::Future,
        _t: PhantomData<Io1>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseB<T, Io1, Io2>
    where
        T: Service<Connect, Response = (Connect, Io2), Error = ConnectorError>,
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
    {
        type Item = IoEither<IoConnection<Io1>, IoConnection<Io2>>;
        type Error = ConnectorError;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            match self.fut.poll()? {
                Async::NotReady => Ok(Async::NotReady),
                Async::Ready(res) => Ok(Async::Ready(IoEither::B(res))),
            }
        }
    }
}

pub(crate) enum IoEither<Io1, Io2> {
    A(Io1),
    B(Io2),
}

impl<Io1, Io2> Connection for IoEither<Io1, Io2>
where
    Io1: Connection,
    Io2: Connection,
{
    fn close(&mut self) {
        match self {
            IoEither::A(ref mut io) => io.close(),
            IoEither::B(ref mut io) => io.close(),
        }
    }

    fn release(&mut self) {
        match self {
            IoEither::A(ref mut io) => io.release(),
            IoEither::B(ref mut io) => io.release(),
        }
    }
}

impl<Io1, Io2> io::Read for IoEither<Io1, Io2>
where
    Io1: Connection,
    Io2: Connection,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            IoEither::A(ref mut io) => io.read(buf),
            IoEither::B(ref mut io) => io.read(buf),
        }
    }
}

impl<Io1, Io2> AsyncRead for IoEither<Io1, Io2>
where
    Io1: Connection,
    Io2: Connection,
{
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match self {
            IoEither::A(ref io) => io.prepare_uninitialized_buffer(buf),
            IoEither::B(ref io) => io.prepare_uninitialized_buffer(buf),
        }
    }
}

impl<Io1, Io2> AsyncWrite for IoEither<Io1, Io2>
where
    Io1: Connection,
    Io2: Connection,
{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match self {
            IoEither::A(ref mut io) => io.shutdown(),
            IoEither::B(ref mut io) => io.shutdown(),
        }
    }

    fn poll_write(&mut self, buf: &[u8]) -> Poll<usize, io::Error> {
        match self {
            IoEither::A(ref mut io) => io.poll_write(buf),
            IoEither::B(ref mut io) => io.poll_write(buf),
        }
    }

    fn poll_flush(&mut self) -> Poll<(), io::Error> {
        match self {
            IoEither::A(ref mut io) => io.poll_flush(),
            IoEither::B(ref mut io) => io.poll_flush(),
        }
    }
}

impl<Io1, Io2> io::Write for IoEither<Io1, Io2>
where
    Io1: Connection,
    Io2: Connection,
{
    fn flush(&mut self) -> io::Result<()> {
        match self {
            IoEither::A(ref mut io) => io.flush(),
            IoEither::B(ref mut io) => io.flush(),
        }
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            IoEither::A(ref mut io) => io.write(buf),
            IoEither::B(ref mut io) => io.write(buf),
        }
    }
}

impl<Io1, Io2> fmt::Debug for IoEither<Io1, Io2>
where
    Io1: fmt::Debug,
    Io2: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            IoEither::A(ref io) => io.fmt(fmt),
            IoEither::B(ref io) => io.fmt(fmt),
        }
    }
}

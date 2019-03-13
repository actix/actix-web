use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use actix_codec::{AsyncRead, AsyncWrite};
use actix_connect::{default_connector, Stream};
use actix_service::{apply_fn, Service, ServiceExt};
use actix_utils::timeout::{TimeoutError, TimeoutService};
use tokio_tcp::TcpStream;

use super::connect::Connect;
use super::connection::Connection;
use super::error::ConnectError;
use super::pool::{ConnectionPool, Protocol};

#[cfg(feature = "ssl")]
use openssl::ssl::SslConnector;

#[cfg(not(feature = "ssl"))]
type SslConnector = ();

/// Http client connector builde instance.
/// `Connector` type uses builder-like pattern for connector service construction.
pub struct Connector<T, U> {
    connector: T,
    timeout: Duration,
    conn_lifetime: Duration,
    conn_keep_alive: Duration,
    disconnect_timeout: Duration,
    limit: usize,
    #[allow(dead_code)]
    ssl: SslConnector,
    _t: PhantomData<U>,
}

impl Connector<(), ()> {
    pub fn new() -> Connector<
        impl Service<
                Request = actix_connect::Connect,
                Response = Stream<TcpStream>,
                Error = actix_connect::ConnectError,
            > + Clone,
        TcpStream,
    > {
        let ssl = {
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
            ssl,
            connector: default_connector(),
            timeout: Duration::from_secs(1),
            conn_lifetime: Duration::from_secs(75),
            conn_keep_alive: Duration::from_secs(15),
            disconnect_timeout: Duration::from_millis(3000),
            limit: 100,
            _t: PhantomData,
        }
    }
}

impl<T, U> Connector<T, U> {
    /// Use custom connector.
    pub fn connector<T1, U1>(self, connector: T1) -> Connector<T1, U1>
    where
        U1: AsyncRead + AsyncWrite + fmt::Debug,
        T1: Service<
                Request = actix_connect::Connect,
                Response = Stream<U1>,
                Error = actix_connect::ConnectError,
            > + Clone,
    {
        Connector {
            connector,
            timeout: self.timeout,
            conn_lifetime: self.conn_lifetime,
            conn_keep_alive: self.conn_keep_alive,
            disconnect_timeout: self.disconnect_timeout,
            limit: self.limit,
            ssl: self.ssl,
            _t: PhantomData,
        }
    }
}

impl<T, U> Connector<T, U>
where
    U: AsyncRead + AsyncWrite + fmt::Debug + 'static,
    T: Service<
            Request = actix_connect::Connect,
            Response = Stream<U>,
            Error = actix_connect::ConnectError,
        > + Clone,
{
    /// Connection timeout, i.e. max time to connect to remote host including dns name resolution.
    /// Set to 1 second by default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[cfg(feature = "ssl")]
    /// Use custom `SslConnector` instance.
    pub fn ssl(mut self, connector: SslConnector) -> Self {
        self.ssl = connector;
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
    ) -> impl Service<Request = Connect, Response = impl Connection, Error = ConnectError>
                 + Clone {
        #[cfg(not(feature = "ssl"))]
        {
            let connector = TimeoutService::new(
                self.timeout,
                self.connector
                    .map(|stream| (stream.into_parts().0, Protocol::Http1)),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
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
            use actix_connect::ssl::OpensslConnector;

            let ssl_service = TimeoutService::new(
                self.timeout,
                apply_fn(self.connector.clone(), |msg: Connect, srv| {
                    srv.call(actix_connect::Connect::new(msg.host(), msg.port()))
                })
                .map_err(ConnectError::from)
                .and_then(
                    OpensslConnector::service(self.ssl)
                        .map_err(ConnectError::from)
                        .map(|stream| {
                            let sock = stream.into_parts().0;
                            let h2 = sock
                                .get_ref()
                                .ssl()
                                .selected_alpn_protocol()
                                .map(|protos| protos.windows(2).any(|w| w == H2))
                                .unwrap_or(false);
                            if h2 {
                                (sock, Protocol::Http2)
                            } else {
                                (sock, Protocol::Http1)
                            }
                        }),
                ),
            )
            .map_err(|e| match e {
                TimeoutError::Service(e) => e,
                TimeoutError::Timeout => ConnectError::Timeout,
            });

            let tcp_service = TimeoutService::new(
                self.timeout,
                apply_fn(self.connector.clone(), |msg: Connect, srv| {
                    srv.call(actix_connect::Connect::new(msg.host(), msg.port()))
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
        T: Service<
            Request = Connect,
            Response = (Connect, Io, Protocol),
            Error = ConnectorError,
        >,
    {
        pub(crate) tcp_pool: ConnectionPool<T, Io>,
    }

    impl<T, Io> Clone for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectError>
            + Clone,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
            }
        }
    }

    impl<T, Io> Service for InnerConnector<T, Io>
    where
        Io: AsyncRead + AsyncWrite + 'static,
        T: Service<Request = Connect, Response = (Io, Protocol), Error = ConnectorError>,
    {
        type Request = Connect;
        type Response = IoConnection<Io>;
        type Error = ConnectorError;
        type Future = Either<
            <ConnectionPool<T, Io> as Service>::Future,
            FutureResult<IoConnection<Io>, ConnectorError>,
        >;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            self.tcp_pool.poll_ready()
        }

        fn call(&mut self, req: Connect) -> Self::Future {
            if req.is_secure() {
                Either::B(err(ConnectError::SslIsNotSupported))
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

    use futures::future::{Either, FutureResult};
    use futures::{Async, Future, Poll};

    use super::*;
    use crate::client::connection::EitherConnection;

    pub(crate) struct InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>,
        T2: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>,
    {
        pub(crate) tcp_pool: ConnectionPool<T1, Io1>,
        pub(crate) ssl_pool: ConnectionPool<T2, Io2>,
    }

    impl<T1, T2, Io1, Io2> Clone for InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>
            + Clone,
        T2: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>
            + Clone,
    {
        fn clone(&self) -> Self {
            InnerConnector {
                tcp_pool: self.tcp_pool.clone(),
                ssl_pool: self.ssl_pool.clone(),
            }
        }
    }

    impl<T1, T2, Io1, Io2> Service for InnerConnector<T1, T2, Io1, Io2>
    where
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
        T1: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>,
        T2: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>,
    {
        type Request = Connect;
        type Response = EitherConnection<Io1, Io2>;
        type Error = ConnectError;
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
            if req.is_secure() {
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
        T: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>,
    {
        fut: <ConnectionPool<T, Io1> as Service>::Future,
        _t: PhantomData<Io2>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseA<T, Io1, Io2>
    where
        T: Service<Request = Connect, Response = (Io1, Protocol), Error = ConnectError>,
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
    {
        type Item = EitherConnection<Io1, Io2>;
        type Error = ConnectError;

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
        T: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>,
    {
        fut: <ConnectionPool<T, Io2> as Service>::Future,
        _t: PhantomData<Io1>,
    }

    impl<T, Io1, Io2> Future for InnerConnectorResponseB<T, Io1, Io2>
    where
        T: Service<Request = Connect, Response = (Io2, Protocol), Error = ConnectError>,
        Io1: AsyncRead + AsyncWrite + 'static,
        Io2: AsyncRead + AsyncWrite + 'static,
    {
        type Item = EitherConnection<Io1, Io2>;
        type Error = ConnectError;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            match self.fut.poll()? {
                Async::NotReady => Ok(Async::NotReady),
                Async::Ready(res) => Ok(Async::Ready(EitherConnection::B(res))),
            }
        }
    }
}

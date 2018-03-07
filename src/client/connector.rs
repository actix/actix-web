use std::{io, time};
use std::net::Shutdown;
use std::time::Duration;

use actix::{fut, Actor, ActorFuture, Context,
            Handler, Message, ActorResponse, Supervised};
use actix::registry::ArbiterService;
use actix::fut::WrapFuture;
use actix::actors::{Connector, ConnectorError, Connect as ResolveConnect};

use http::{Uri, HttpTryFrom, Error as HttpError};
use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};

#[cfg(feature="alpn")]
use openssl::ssl::{SslMethod, SslConnector, Error as OpensslError};
#[cfg(feature="alpn")]
use tokio_openssl::SslConnectorExt;
#[cfg(feature="alpn")]
use futures::Future;

use HAS_OPENSSL;
use server::IoStream;


#[derive(Debug)]
/// `Connect` type represents message that can be send to `ClientConnector`
/// with connection request.
pub struct Connect {
    pub uri: Uri,
    pub conn_timeout: Duration,
}

impl Connect {
    /// Create `Connect` message for specified `Uri`
    pub fn new<U>(uri: U) -> Result<Connect, HttpError> where Uri: HttpTryFrom<U> {
        Ok(Connect {
            uri: Uri::try_from(uri).map_err(|e| e.into())?,
            conn_timeout: Duration::from_secs(1)
        })
    }
}

impl Message for Connect {
    type Result = Result<Connection, ClientConnectorError>;
}

/// A set of errors that can occur during connecting to a http host
#[derive(Fail, Debug)]
pub enum ClientConnectorError {
    /// Invalid url
    #[fail(display="Invalid url")]
    InvalidUrl,

    /// SSL feature is not enabled
    #[fail(display="SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature="alpn")]
    #[fail(display="{}", _0)]
    SslError(#[cause] OpensslError),

    /// Connection error
    #[fail(display = "{}", _0)]
    Connector(#[cause] ConnectorError),

    /// Connecting took too long
    #[fail(display = "Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[fail(display = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Connection io error
    #[fail(display = "{}", _0)]
    IoError(#[cause] io::Error),
}

impl From<ConnectorError> for ClientConnectorError {
    fn from(err: ConnectorError) -> ClientConnectorError {
        ClientConnectorError::Connector(err)
    }
}

pub struct ClientConnector {
    #[cfg(feature="alpn")]
    connector: SslConnector,
}

impl Actor for ClientConnector {
    type Context = Context<ClientConnector>;
}

impl Supervised for ClientConnector {}

impl ArbiterService for ClientConnector {}

impl Default for ClientConnector {
    fn default() -> ClientConnector {
        #[cfg(feature="alpn")]
        {
            let builder = SslConnector::builder(SslMethod::tls()).unwrap();
            ClientConnector {
                connector: builder.build()
            }
        }

        #[cfg(not(feature="alpn"))]
        ClientConnector {}
    }
}

impl ClientConnector {

    #[cfg(feature="alpn")]
    /// Create `ClientConnector` actor with custom `SslConnector` instance.
    ///
    /// By default `ClientConnector` uses very simple ssl configuration.
    /// With `with_connector` method it is possible to use custom `SslConnector`
    /// object.
    ///
    /// ```rust
    /// # #![cfg(feature="alpn")]
    /// # extern crate actix;
    /// # extern crate actix_web;
    /// # extern crate futures;
    /// # use futures::Future;
    /// # use std::io::Write;
    /// extern crate openssl;
    /// use actix::prelude::*;
    /// use actix_web::client::{Connect, ClientConnector};
    ///
    /// use openssl::ssl::{SslMethod, SslConnector};
    ///
    /// fn main() {
    ///     let sys = System::new("test");
    ///
    ///     // Start `ClientConnector` with custom `SslConnector`
    ///     let ssl_conn = SslConnector::builder(SslMethod::tls()).unwrap().build();
    ///     let conn: Address<_> = ClientConnector::with_connector(ssl_conn).start();
    ///
    ///     Arbiter::handle().spawn({
    ///         conn.send(
    ///             Connect::new("https://www.rust-lang.org").unwrap()) // <- connect to host
    ///                 .map_err(|_| ())
    ///                 .and_then(|res| {
    ///                     if let Ok(mut stream) = res {
    ///                         stream.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
    ///                     }
    /// #                   Arbiter::system().do_send(actix::msgs::SystemExit(0));
    ///                     Ok(())
    ///                 })
    ///     });
    ///
    ///     sys.run();
    /// }
    /// ```
    pub fn with_connector(connector: SslConnector) -> ClientConnector {
        ClientConnector { connector }
    }
}

impl Handler<Connect> for ClientConnector {
    type Result = ActorResponse<ClientConnector, Connection, ClientConnectorError>;

    fn handle(&mut self, msg: Connect, _: &mut Self::Context) -> Self::Result {
        let uri = &msg.uri;
        let conn_timeout = msg.conn_timeout;

        // host name is required
        if uri.host().is_none() {
            return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl))
        }

        // supported protocols
        let proto = match uri.scheme_part() {
            Some(scheme) => match Protocol::from(scheme.as_str()) {
                Some(proto) => proto,
                None => return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl)),
            },
            None => return ActorResponse::reply(Err(ClientConnectorError::InvalidUrl)),
        };

        // check ssl availability
        if proto.is_secure() && !HAS_OPENSSL { //&& !HAS_TLS {
            return ActorResponse::reply(Err(ClientConnectorError::SslIsNotSupported))
        }

        let host = uri.host().unwrap().to_owned();
        let port = uri.port().unwrap_or_else(|| proto.port());

        ActorResponse::async(
            Connector::from_registry()
                .send(ResolveConnect::host_and_port(&host, port)
                      .timeout(conn_timeout))
                .into_actor(self)
                .map_err(|_, _, _| ClientConnectorError::Disconnected)
                .and_then(move |res, _act, _| {
                    #[cfg(feature="alpn")]
                    match res {
                        Err(err) => fut::Either::B(fut::err(err.into())),
                        Ok(stream) => {
                            if proto.is_secure() {
                                fut::Either::A(
                                    _act.connector.connect_async(&host, stream)
                                        .map_err(ClientConnectorError::SslError)
                                        .map(|stream| Connection{stream: Box::new(stream)})
                                        .into_actor(_act))
                            } else {
                                fut::Either::B(fut::ok(Connection{stream: Box::new(stream)}))
                            }
                        }
                    }

                    #[cfg(not(feature="alpn"))]
                    match res {
                        Err(err) => fut::err(err.into()),
                        Ok(stream) => {
                            if proto.is_secure() {
                                fut::err(ClientConnectorError::SslIsNotSupported)
                            } else {
                                fut::ok(Connection{stream: Box::new(stream)})
                            }
                        }
                    }
                }))
    }
}

#[derive(PartialEq, Hash, Debug, Clone, Copy)]
enum Protocol {
    Http,
    Https,
    Ws,
    Wss,
}

impl Protocol {
    fn from(s: &str) -> Option<Protocol> {
        match s {
            "http" => Some(Protocol::Http),
            "https" => Some(Protocol::Https),
            "ws" => Some(Protocol::Ws),
            "wss" => Some(Protocol::Wss),
            _ => None,
        }
    }

    fn is_secure(&self) -> bool {
        match *self {
            Protocol::Https | Protocol::Wss => true,
            _ => false,
        }
    }

    fn port(&self) -> u16 {
        match *self {
            Protocol::Http | Protocol::Ws => 80,
            Protocol::Https | Protocol::Wss => 443
        }
    }
}


pub struct Connection {
    stream: Box<IoStream>,
}

impl Connection {
    pub fn stream(&mut self) -> &mut IoStream {
        &mut *self.stream
    }

    pub fn from_stream<T: IoStream>(io: T) -> Connection {
        Connection{stream: Box::new(io)}
    }
}

impl IoStream for Connection {
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        IoStream::shutdown(&mut *self.stream, how)
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        IoStream::set_nodelay(&mut *self.stream, nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        IoStream::set_linger(&mut *self.stream, dur)
    }
}

impl io::Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl AsyncRead for Connection {}

impl io::Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl AsyncWrite for Connection {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.stream.shutdown()
    }
}

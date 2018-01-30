#![allow(unused_imports, dead_code)]
use std::{io, time};
use std::net::{SocketAddr, Shutdown};
use std::collections::VecDeque;
use std::time::Duration;

use actix::{fut, Actor, ActorFuture, Arbiter, ArbiterService, Context,
            Handler, Response, ResponseType, Supervised};
use actix::actors::{Connector, ConnectorError, Connect as ResolveConnect};

use http::Uri;
use futures::{Async, Future, Poll};
use tokio_core::reactor::Timeout;
use tokio_core::net::{TcpStream, TcpStreamNew};
use tokio_io::{AsyncRead, AsyncWrite};

use server::IoStream;

#[derive(Debug)]
pub struct Connect(pub Uri);

impl ResponseType for Connect {
    type Item = Connection;
    type Error = ClientConnectorError;
}

#[derive(Fail, Debug)]
pub enum ClientConnectorError {
    /// Invalid url
    #[fail(display="Invalid url")]
    InvalidUrl,

    /// SSL feature is not enabled
    #[fail(display="SSL is not supported")]
    SslIsNotSupported,

    /// Connection error
    #[fail(display = "{}", _0)]
    Connector(ConnectorError),

    /// Connecting took too long
    #[fail(display = "Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[fail(display = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Connection io error
    #[fail(display = "{}", _0)]
    IoError(io::Error),
}

impl From<ConnectorError> for ClientConnectorError {
    fn from(err: ConnectorError) -> ClientConnectorError {
        ClientConnectorError::Connector(err)
    }
}

#[derive(Debug, Default)]
pub struct ClientConnector {
}

impl Actor for ClientConnector {
    type Context = Context<ClientConnector>;
}

impl Supervised for ClientConnector {}

impl ArbiterService for ClientConnector {}

impl Handler<Connect> for ClientConnector {
    type Result = Response<ClientConnector, Connect>;

    fn handle(&mut self, msg: Connect, _: &mut Self::Context) -> Self::Result {
        let uri = &msg.0;

        if uri.host().is_none() {
            return Self::reply(Err(ClientConnectorError::InvalidUrl))
        }

        let proto = match uri.scheme_part() {
            Some(scheme) => match Protocol::from(scheme.as_str()) {
                Some(proto) => proto,
                None => return Self::reply(Err(ClientConnectorError::InvalidUrl)),
            },
            None => return Self::reply(Err(ClientConnectorError::InvalidUrl)),
        };

        let port = uri.port().unwrap_or_else(|| proto.port());

        Self::async_reply(
            Connector::from_registry()
                .call(self, ResolveConnect::host_and_port(uri.host().unwrap(), port))
                .map_err(|_, _, _| ClientConnectorError::Disconnected)
                .and_then(|res, _, _| match res {
                    Ok(stream) => fut::ok(Connection{stream: Box::new(stream)}),
                    Err(err) => fut::err(err.into())
                }))
    }
}

#[derive(PartialEq, Hash, Debug)]
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

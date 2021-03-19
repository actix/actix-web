//! HTTP client.

use http::Uri;

mod config;
mod connection;
mod connector;
mod error;
mod h1proto;
mod h2proto;
mod pool;

pub use actix_tls::connect::{
    Connect as TcpConnect, ConnectError as TcpConnectError, Connection as TcpConnection,
};

pub use self::connection::{Connection, ConnectionIo};
pub use self::connector::{Connector, ConnectorService};
pub use self::error::{ConnectError, FreezeRequestError, InvalidUrl, SendRequestError};
pub use crate::Protocol;

#[derive(Clone)]
pub struct Connect {
    pub uri: Uri,
    pub addr: Option<std::net::SocketAddr>,
}

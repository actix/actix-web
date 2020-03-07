//! Http client api
use http::Uri;

mod config;
mod connection;
mod connector;
mod error;
mod h1proto;
mod h2proto;
mod pool;

pub use self::connection::Connection;
pub use self::connector::Connector;
pub use self::error::{ConnectError, FreezeRequestError, InvalidUrl, SendRequestError};
pub use self::pool::Protocol;

#[derive(Clone)]
pub struct Connect {
    pub uri: Uri,
    pub addr: Option<std::net::SocketAddr>,
}

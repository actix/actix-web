//! Actix Connector - tcp connector service
//!
//! ## Package feature
//!
//! * `tls` - enables ssl support via `native-tls` crate
//! * `ssl` - enables ssl support via `openssl` crate
//! * `rust-tls` - enables ssl support via `rustls` crate

mod connector;
mod resolver;
pub mod ssl;

pub use self::connector::{
    Connect, Connector, ConnectorError, DefaultConnector, RequestPort, TcpConnector,
};
pub use self::resolver::{RequestHost, Resolver};

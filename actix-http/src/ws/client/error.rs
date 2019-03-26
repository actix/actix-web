//! Http client request
use std::io;

use actix_connect::ConnectError;
use derive_more::{Display, From};
use http::{header::HeaderValue, Error as HttpError, StatusCode};

use crate::error::ParseError;
use crate::ws::ProtocolError;

/// Websocket client error
#[derive(Debug, Display, From)]
pub enum ClientError {
    /// Invalid url
    #[display(fmt = "Invalid url")]
    InvalidUrl,
    /// Invalid response status
    #[display(fmt = "Invalid response status")]
    InvalidResponseStatus(StatusCode),
    /// Invalid upgrade header
    #[display(fmt = "Invalid upgrade header")]
    InvalidUpgradeHeader,
    /// Invalid connection header
    #[display(fmt = "Invalid connection header")]
    InvalidConnectionHeader(HeaderValue),
    /// Missing CONNECTION header
    #[display(fmt = "Missing CONNECTION header")]
    MissingConnectionHeader,
    /// Missing SEC-WEBSOCKET-ACCEPT header
    #[display(fmt = "Missing SEC-WEBSOCKET-ACCEPT header")]
    MissingWebSocketAcceptHeader,
    /// Invalid challenge response
    #[display(fmt = "Invalid challenge response")]
    InvalidChallengeResponse(String, HeaderValue),
    /// Http parsing error
    #[display(fmt = "Http parsing error")]
    Http(HttpError),
    /// Response parsing error
    #[display(fmt = "Response parsing error: {}", _0)]
    ParseError(ParseError),
    /// Protocol error
    #[display(fmt = "{}", _0)]
    Protocol(ProtocolError),
    /// Connect error
    #[display(fmt = "Connector error: {:?}", _0)]
    Connect(ConnectError),
    /// IO Error
    #[display(fmt = "{}", _0)]
    Io(io::Error),
    /// "Disconnected"
    #[display(fmt = "Disconnected")]
    Disconnected,
}

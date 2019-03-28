//! Http client errors
pub use actix_http::client::{ConnectError, InvalidUrl, SendRequestError};
pub use actix_http::error::PayloadError;
pub use actix_http::ws::ProtocolError as WsProtocolError;

use actix_http::http::{header::HeaderValue, Error as HttpError, StatusCode};
use derive_more::{Display, From};

/// Websocket client error
#[derive(Debug, Display, From)]
pub enum WsClientError {
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
    /// Protocol error
    #[display(fmt = "{}", _0)]
    Protocol(WsProtocolError),
    /// Send request error
    #[display(fmt = "{}", _0)]
    SendRequest(SendRequestError),
}

impl From<InvalidUrl> for WsClientError {
    fn from(err: InvalidUrl) -> Self {
        WsClientError::SendRequest(err.into())
    }
}

impl From<HttpError> for WsClientError {
    fn from(err: HttpError) -> Self {
        WsClientError::SendRequest(err.into())
    }
}

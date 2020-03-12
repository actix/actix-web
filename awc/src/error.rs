//! Http client errors
pub use actix_http::client::{
    ConnectError, FreezeRequestError, InvalidUrl, SendRequestError,
};
pub use actix_http::error::PayloadError;
pub use actix_http::http::Error as HttpError;
pub use actix_http::ws::HandshakeError as WsHandshakeError;
pub use actix_http::ws::ProtocolError as WsProtocolError;

use actix_http::ResponseError;
use serde_json::error::Error as JsonError;

use actix_http::http::{header::HeaderValue, StatusCode};
use thiserror::Error;

/// Websocket client error
#[derive(Debug, Error)]
pub enum WsClientError {
    /// Invalid response status
    #[error("Invalid response status")]
    InvalidResponseStatus(StatusCode),
    /// Invalid upgrade header
    #[error("Invalid upgrade header")]
    InvalidUpgradeHeader,
    /// Invalid connection header
    #[error("Invalid connection header")]
    InvalidConnectionHeader(HeaderValue),
    /// Missing CONNECTION header
    #[error("Missing CONNECTION header")]
    MissingConnectionHeader,
    /// Missing SEC-WEBSOCKET-ACCEPT header
    #[error("Missing SEC-WEBSOCKET-ACCEPT header")]
    MissingWebSocketAcceptHeader,
    /// Invalid challenge response
    #[error("Invalid challenge response")]
    InvalidChallengeResponse(String, HeaderValue),
    /// Protocol error
    #[error(transparent)]
    Protocol(#[from] WsProtocolError),
    /// Send request error
    #[error(transparent)]
    SendRequest(#[from] SendRequestError),
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

/// A set of errors that can occur during parsing json payloads
#[derive(Debug, Error)]
pub enum JsonPayloadError {
    /// Content type error
    #[error("Content type error")]
    ContentType,
    /// Deserialize error
    #[error("Json deserialize error: {0}")]
    Deserialize(#[from] JsonError),
    /// Payload error
    #[error("Error that occur during reading payload: {0}")]
    Payload(#[from] PayloadError),
}

/// Return `InternalServerError` for `JsonPayloadError`
impl ResponseError for JsonPayloadError {}

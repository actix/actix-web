use std::io;

use actix_connect::resolver::ResolveError;
use thiserror::Error;

#[cfg(feature = "openssl")]
use actix_connect::ssl::openssl::{HandshakeError, SslError};

use crate::error::{Error, ParseError, ResponseError};
use crate::http::{Error as HttpError, StatusCode};

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Debug, Error)]
pub enum ConnectError {
    /// SSL feature is not enabled
    #[error("SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "openssl")]
    #[error(transparent)]
    SslError(#[from] SslError),

    /// SSL Handshake error
    #[cfg(feature = "openssl")]
    #[error("SSL handshake error: {0}")]
    SslHandshakeError(String),

    /// Failed to resolve the hostname
    #[error("Failed resolving hostname: {0}")]
    Resolver(#[from] ResolveError),

    /// No dns records
    #[error("No dns records found for the input")]
    NoRecords,

    /// Http2 error
    #[error(transparent)]
    H2(#[from] h2::Error),

    /// Connecting took too long
    #[error("Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[error("Internal error: connector has been disconnected")]
    Disconnected,

    /// Unresolved host name
    #[error("Connector received `Connect` method with unresolved host")]
    Unresolverd,

    /// Connection io error
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<actix_connect::ConnectError> for ConnectError {
    fn from(err: actix_connect::ConnectError) -> ConnectError {
        match err {
            actix_connect::ConnectError::Resolver(e) => ConnectError::Resolver(e),
            actix_connect::ConnectError::NoRecords => ConnectError::NoRecords,
            actix_connect::ConnectError::InvalidInput => panic!(),
            actix_connect::ConnectError::Unresolverd => ConnectError::Unresolverd,
            actix_connect::ConnectError::Io(e) => ConnectError::Io(e),
        }
    }
}

#[cfg(feature = "openssl")]
impl<T: std::fmt::Debug> From<HandshakeError<T>> for ConnectError {
    fn from(err: HandshakeError<T>) -> ConnectError {
        ConnectError::SslHandshakeError(format!("{:?}", err))
    }
}

#[derive(Debug, Error)]
pub enum InvalidUrl {
    #[error("Missing url scheme")]
    MissingScheme,
    #[error("Unknown url scheme")]
    UnknownScheme,
    #[error("Missing host name")]
    MissingHost,
    #[error("Url parse error: {0}")]
    HttpError(#[from] http::Error),
}

/// A set of errors that can occur during request sending and response reading
#[derive(Debug, Error)]
pub enum SendRequestError {
    /// Invalid URL
    #[error("Invalid URL: {0}")]
    Url(#[from] InvalidUrl),
    /// Failed to connect to host
    #[error("Failed to connect to host: {0}")]
    Connect(#[from] ConnectError),
    /// Error sending request
    #[error(transparent)]
    Send(#[from] io::Error),
    /// Error parsing response
    #[error(transparent)]
    Response(#[from] ParseError),
    /// Http error
    #[error(transparent)]
    Http(#[from] HttpError),
    /// Http2 error
    #[error(transparent)]
    H2(#[from] h2::Error),
    /// Response took too long
    #[error("Timeout out while waiting for response")]
    Timeout,
    /// Tunnels are not supported for http2 connection
    #[error("Tunnels are not supported for http2 connection")]
    TunnelNotSupported,
    /// Error sending request body
    #[error(transparent)]
    Body(#[from] Error),
}

/// Convert `SendRequestError` to a server `Response`
impl ResponseError for SendRequestError {
    fn status_code(&self) -> StatusCode {
        match *self {
            SendRequestError::Connect(ConnectError::Timeout) => {
                StatusCode::GATEWAY_TIMEOUT
            }
            SendRequestError::Connect(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// A set of errors that can occur during freezing a request
#[derive(Debug, Error)]
pub enum FreezeRequestError {
    /// Invalid URL
    #[error("Invalid URL: {0}")]
    Url(#[from] InvalidUrl),
    /// Http error
    #[error(transparent)]
    Http(#[from] HttpError),
}

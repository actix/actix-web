use std::{fmt, io};

use derive_more::{Display, From};

use actix_http::error::{HttpError, ParseError};

#[cfg(feature = "openssl")]
use actix_tls::accept::openssl::reexports::Error as OpensslError;

use crate::BoxError;

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum ConnectError {
    /// SSL feature is not enabled
    #[display(fmt = "SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "openssl")]
    #[display(fmt = "{}", _0)]
    SslError(OpensslError),

    /// Failed to resolve the hostname
    #[display(fmt = "Failed resolving hostname: {}", _0)]
    Resolver(Box<dyn std::error::Error>),

    /// No dns records
    #[display(fmt = "No DNS records found for the input")]
    NoRecords,

    /// Http2 error
    #[display(fmt = "{}", _0)]
    H2(h2::Error),

    /// Connecting took too long
    #[display(fmt = "Timeout while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[display(fmt = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Unresolved host name
    #[display(fmt = "Connector received `Connect` method with unresolved host")]
    Unresolved,

    /// Connection io error
    #[display(fmt = "{}", _0)]
    Io(io::Error),
}

impl std::error::Error for ConnectError {}

impl From<actix_tls::connect::ConnectError> for ConnectError {
    fn from(err: actix_tls::connect::ConnectError) -> ConnectError {
        match err {
            actix_tls::connect::ConnectError::Resolver(e) => ConnectError::Resolver(e),
            actix_tls::connect::ConnectError::NoRecords => ConnectError::NoRecords,
            actix_tls::connect::ConnectError::InvalidInput => panic!(),
            actix_tls::connect::ConnectError::Unresolved => ConnectError::Unresolved,
            actix_tls::connect::ConnectError::Io(e) => ConnectError::Io(e),
        }
    }
}

#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum InvalidUrl {
    #[display(fmt = "Missing URL scheme")]
    MissingScheme,

    #[display(fmt = "Unknown URL scheme")]
    UnknownScheme,

    #[display(fmt = "Missing host name")]
    MissingHost,

    #[display(fmt = "URL parse error: {}", _0)]
    HttpError(http::Error),
}

impl std::error::Error for InvalidUrl {}

/// A set of errors that can occur during request sending and response reading
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum SendRequestError {
    /// Invalid URL
    #[display(fmt = "Invalid URL: {}", _0)]
    Url(InvalidUrl),

    /// Failed to connect to host
    #[display(fmt = "Failed to connect to host: {}", _0)]
    Connect(ConnectError),

    /// Error sending request
    Send(io::Error),

    /// Error parsing response
    Response(ParseError),

    /// Http error
    #[display(fmt = "{}", _0)]
    Http(HttpError),

    /// Http2 error
    #[display(fmt = "{}", _0)]
    H2(h2::Error),

    /// Response took too long
    #[display(fmt = "Timeout while waiting for response")]
    Timeout,

    /// Tunnels are not supported for HTTP/2 connection
    #[display(fmt = "Tunnels are not supported for http2 connection")]
    TunnelNotSupported,

    /// Error sending request body
    Body(BoxError),

    /// Other errors that can occur after submitting a request.
    #[display(fmt = "{:?}: {}", _1, _0)]
    Custom(BoxError, Box<dyn fmt::Debug>),
}

impl std::error::Error for SendRequestError {}

/// A set of errors that can occur during freezing a request
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum FreezeRequestError {
    /// Invalid URL
    #[display(fmt = "Invalid URL: {}", _0)]
    Url(InvalidUrl),

    /// HTTP error
    #[display(fmt = "{}", _0)]
    Http(HttpError),

    /// Other errors that can occur after submitting a request.
    #[display(fmt = "{:?}: {}", _1, _0)]
    Custom(BoxError, Box<dyn fmt::Debug>),
}

impl std::error::Error for FreezeRequestError {}

impl From<FreezeRequestError> for SendRequestError {
    fn from(err: FreezeRequestError) -> Self {
        match err {
            FreezeRequestError::Url(err) => err.into(),
            FreezeRequestError::Http(err) => err.into(),
            FreezeRequestError::Custom(err, msg) => SendRequestError::Custom(err, msg),
        }
    }
}

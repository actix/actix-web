use std::{fmt, io};

use actix_http::error::{HttpError, ParseError};
#[cfg(feature = "openssl")]
use actix_tls::accept::openssl::reexports::Error as OpensslError;
use derive_more::{Display, From};

use crate::BoxError;

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum ConnectError {
    /// SSL feature is not enabled
    #[display("SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "openssl")]
    #[display("{}", _0)]
    SslError(OpensslError),

    /// Failed to resolve the hostname
    #[display("Failed resolving hostname: {}", _0)]
    Resolver(Box<dyn std::error::Error>),

    /// No dns records
    #[display("No DNS records found for the input")]
    NoRecords,

    /// Http2 error
    #[display("{}", _0)]
    H2(h2::Error),

    /// Connecting took too long
    #[display("Timeout while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[display("Internal error: connector has been disconnected")]
    Disconnected,

    /// Unresolved host name
    #[display("Connector received `Connect` method with unresolved host")]
    Unresolved,

    /// Connection io error
    #[display("{}", _0)]
    Io(io::Error),
}

impl std::error::Error for ConnectError {}

impl From<actix_tls::connect::ConnectError> for ConnectError {
    fn from(err: actix_tls::connect::ConnectError) -> ConnectError {
        match err {
            actix_tls::connect::ConnectError::Resolver(err) => ConnectError::Resolver(err),
            actix_tls::connect::ConnectError::NoRecords => ConnectError::NoRecords,
            actix_tls::connect::ConnectError::InvalidInput => panic!(),
            actix_tls::connect::ConnectError::Unresolved => ConnectError::Unresolved,
            actix_tls::connect::ConnectError::Io(err) => ConnectError::Io(err),
        }
    }
}

#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum InvalidUrl {
    #[display("Missing URL scheme")]
    MissingScheme,

    #[display("Unknown URL scheme")]
    UnknownScheme,

    #[display("Missing host name")]
    MissingHost,

    #[display("URL parse error: {}", _0)]
    HttpError(http::Error),
}

impl std::error::Error for InvalidUrl {}

/// A set of errors that can occur during request sending and response reading
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum SendRequestError {
    /// Invalid URL
    #[display("Invalid URL: {}", _0)]
    Url(InvalidUrl),

    /// Failed to connect to host
    #[display("Failed to connect to host: {}", _0)]
    Connect(ConnectError),

    /// Error sending request
    Send(io::Error),

    /// Error parsing response
    Response(ParseError),

    /// Http error
    #[display("{}", _0)]
    Http(HttpError),

    /// Http2 error
    #[display("{}", _0)]
    H2(h2::Error),

    /// Response took too long
    #[display("Timeout while waiting for response")]
    Timeout,

    /// Tunnels are not supported for HTTP/2 connection
    #[display("Tunnels are not supported for http2 connection")]
    TunnelNotSupported,

    /// Error sending request body
    Body(BoxError),

    /// Other errors that can occur after submitting a request.
    #[display("{:?}: {}", _1, _0)]
    Custom(BoxError, Box<dyn fmt::Debug>),
}

impl std::error::Error for SendRequestError {}

/// A set of errors that can occur during freezing a request
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum FreezeRequestError {
    /// Invalid URL
    #[display("Invalid URL: {}", _0)]
    Url(InvalidUrl),

    /// HTTP error
    #[display("{}", _0)]
    Http(HttpError),

    /// Other errors that can occur after submitting a request.
    #[display("{:?}: {}", _1, _0)]
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

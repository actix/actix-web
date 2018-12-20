use std::io;

use derive_more::{Display, From};
use trust_dns_resolver::error::ResolveError;

#[cfg(feature = "ssl")]
use openssl::ssl::{Error as SslError, HandshakeError};

use crate::error::{Error, ParseError};

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Debug, Display, From)]
pub enum ConnectorError {
    /// Invalid URL
    #[display(fmt = "Invalid URL")]
    InvalidUrl(InvalidUrlKind),

    /// SSL feature is not enabled
    #[display(fmt = "SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "ssl")]
    #[display(fmt = "{}", _0)]
    SslError(SslError),

    /// Failed to resolve the hostname
    #[display(fmt = "Failed resolving hostname: {}", _0)]
    Resolver(ResolveError),

    /// No dns records
    #[display(fmt = "No dns records found for the input")]
    NoRecords,

    /// Connecting took too long
    #[display(fmt = "Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[display(fmt = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Connection io error
    #[display(fmt = "{}", _0)]
    Io(io::Error),
}

#[derive(Debug, Display)]
pub enum InvalidUrlKind {
    #[display(fmt = "Missing url scheme")]
    MissingScheme,
    #[display(fmt = "Unknown url scheme")]
    UnknownScheme,
    #[display(fmt = "Missing host name")]
    MissingHost,
}

#[cfg(feature = "ssl")]
impl<T> From<HandshakeError<T>> for ConnectorError {
    fn from(err: HandshakeError<T>) -> ConnectorError {
        match err {
            HandshakeError::SetupFailure(stack) => SslError::from(stack).into(),
            HandshakeError::Failure(stream) => {
                SslError::from(stream.into_error()).into()
            }
            HandshakeError::WouldBlock(stream) => {
                SslError::from(stream.into_error()).into()
            }
        }
    }
}

/// A set of errors that can occur during request sending and response reading
#[derive(Debug, Display, From)]
pub enum SendRequestError {
    /// Failed to connect to host
    #[display(fmt = "Failed to connect to host: {}", _0)]
    Connector(ConnectorError),
    /// Error sending request
    Send(io::Error),
    /// Error parsing response
    Response(ParseError),
    /// Error sending request body
    Body(Error),
}

use std::io;

use failure::Fail;
use trust_dns_resolver::error::ResolveError;

#[cfg(feature = "ssl")]
use openssl::ssl::{Error as SslError, HandshakeError};

use crate::error::{Error, ParseError};

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Fail, Debug)]
pub enum ConnectorError {
    /// Invalid URL
    #[fail(display = "Invalid URL")]
    InvalidUrl(InvalidUrlKind),

    /// SSL feature is not enabled
    #[fail(display = "SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "ssl")]
    #[fail(display = "{}", _0)]
    SslError(#[cause] SslError),

    /// Failed to resolve the hostname
    #[fail(display = "Failed resolving hostname: {}", _0)]
    Resolver(ResolveError),

    /// No dns records
    #[fail(display = "No dns records found for the input")]
    NoRecords,

    /// Connecting took too long
    #[fail(display = "Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[fail(display = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Connection io error
    #[fail(display = "{}", _0)]
    IoError(io::Error),
}

#[derive(Fail, Debug)]
pub enum InvalidUrlKind {
    #[fail(display = "Missing url scheme")]
    MissingScheme,
    #[fail(display = "Unknown url scheme")]
    UnknownScheme,
    #[fail(display = "Missing host name")]
    MissingHost,
}

impl From<io::Error> for ConnectorError {
    fn from(err: io::Error) -> ConnectorError {
        ConnectorError::IoError(err)
    }
}

impl From<ResolveError> for ConnectorError {
    fn from(err: ResolveError) -> ConnectorError {
        ConnectorError::Resolver(err)
    }
}

#[cfg(feature = "ssl")]
impl From<SslError> for ConnectorError {
    fn from(err: SslError) -> ConnectorError {
        ConnectorError::SslError(err)
    }
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
#[derive(Debug)]
pub enum SendRequestError {
    /// Failed to connect to host
    // #[fail(display = "Failed to connect to host: {}", _0)]
    Connector(ConnectorError),
    /// Error sending request
    Send(io::Error),
    /// Error parsing response
    Response(ParseError),
    /// Error sending request body
    Body(Error),
}

impl From<io::Error> for SendRequestError {
    fn from(err: io::Error) -> SendRequestError {
        SendRequestError::Send(err)
    }
}

impl From<ConnectorError> for SendRequestError {
    fn from(err: ConnectorError) -> SendRequestError {
        SendRequestError::Connector(err)
    }
}

impl From<ParseError> for SendRequestError {
    fn from(err: ParseError) -> SendRequestError {
        SendRequestError::Response(err)
    }
}

impl From<Error> for SendRequestError {
    fn from(err: Error) -> SendRequestError {
        SendRequestError::Body(err)
    }
}

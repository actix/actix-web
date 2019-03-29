use std::io;

use derive_more::{Display, From};
use trust_dns_resolver::error::ResolveError;

#[cfg(feature = "ssl")]
use openssl::ssl::{Error as SslError, HandshakeError};

use crate::error::{Error, ParseError, ResponseError};
use crate::http::Error as HttpError;
use crate::response::Response;

/// A set of errors that can occur while connecting to an HTTP host
#[derive(Debug, Display, From)]
pub enum ConnectError {
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

    /// Http2 error
    #[display(fmt = "{}", _0)]
    H2(h2::Error),

    /// Connecting took too long
    #[display(fmt = "Timeout out while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[display(fmt = "Internal error: connector has been disconnected")]
    Disconnected,

    /// Unresolved host name
    #[display(fmt = "Connector received `Connect` method with unresolved host")]
    Unresolverd,

    /// Connection io error
    #[display(fmt = "{}", _0)]
    Io(io::Error),
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

#[cfg(feature = "ssl")]
impl<T> From<HandshakeError<T>> for ConnectError {
    fn from(err: HandshakeError<T>) -> ConnectError {
        match err {
            HandshakeError::SetupFailure(stack) => SslError::from(stack).into(),
            HandshakeError::Failure(stream) => stream.into_error().into(),
            HandshakeError::WouldBlock(stream) => stream.into_error().into(),
        }
    }
}

#[derive(Debug, Display, From)]
pub enum InvalidUrl {
    #[display(fmt = "Missing url scheme")]
    MissingScheme,
    #[display(fmt = "Unknown url scheme")]
    UnknownScheme,
    #[display(fmt = "Missing host name")]
    MissingHost,
    #[display(fmt = "Url parse error: {}", _0)]
    HttpError(http::Error),
}

/// A set of errors that can occur during request sending and response reading
#[derive(Debug, Display, From)]
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
    #[display(fmt = "Timeout out while waiting for response")]
    Timeout,
    /// Tunnels are not supported for http2 connection
    #[display(fmt = "Tunnels are not supported for http2 connection")]
    TunnelNotSupported,
    /// Error sending request body
    Body(Error),
}

/// Convert `SendRequestError` to a server `Response`
impl ResponseError for SendRequestError {
    fn error_response(&self) -> Response {
        match *self {
            SendRequestError::Connect(ConnectError::Timeout) => {
                Response::GatewayTimeout()
            }
            SendRequestError::Connect(_) => Response::BadGateway(),
            _ => Response::InternalServerError(),
        }
        .into()
    }
}

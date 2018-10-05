use std::fmt::{Debug, Display};
use std::io;

use futures::{Async, Poll};

use error::{Error, ParseError};
use http::{StatusCode, Version};

/// Errors produced by `AcceptorError` service.
#[derive(Debug)]
pub enum AcceptorError<T> {
    /// The inner service error
    Service(T),

    /// Io specific error
    Io(io::Error),

    /// The request did not complete within the specified timeout.
    Timeout,
}

#[derive(Debug)]
/// A set of errors that can occur during dispatching http requests
pub enum HttpDispatchError<E: Debug + Display> {
    /// Application error
    // #[fail(display = "Application specific error: {}", _0)]
    App(E),

    /// An `io::Error` that occurred while trying to read or write to a network
    /// stream.
    // #[fail(display = "IO error: {}", _0)]
    Io(io::Error),

    /// Http request parse error.
    // #[fail(display = "Parse error: {}", _0)]
    Parse(ParseError),

    /// The first request did not complete within the specified timeout.
    // #[fail(display = "The first request did not complete within the specified timeout")]
    SlowRequestTimeout,

    /// Shutdown timeout
    // #[fail(display = "Connection shutdown timeout")]
    ShutdownTimeout,

    /// Payload is not consumed
    // #[fail(display = "Task is completed but request's payload is not consumed")]
    PayloadIsNotConsumed,

    /// Malformed request
    // #[fail(display = "Malformed request")]
    MalformedRequest,

    /// Internal error
    // #[fail(display = "Internal error")]
    InternalError,

    /// Unknown error
    // #[fail(display = "Unknown error")]
    Unknown,
}

impl<E: Debug + Display> From<ParseError> for HttpDispatchError<E> {
    fn from(err: ParseError) -> Self {
        HttpDispatchError::Parse(err)
    }
}

impl<E: Debug + Display> From<io::Error> for HttpDispatchError<E> {
    fn from(err: io::Error) -> Self {
        HttpDispatchError::Io(err)
    }
}

use std::fmt::{Debug, Display};
use std::io;

use futures::{Async, Poll};
use http2;

use super::{helpers, HttpHandlerTask, Writer};
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

    /// HTTP2 error
    // #[fail(display = "HTTP2 error: {}", _0)]
    Http2(http2::Error),

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

// impl<E: Debug + Display> From<E> for HttpDispatchError<E> {
//     fn from(err: E) -> Self {
//         HttpDispatchError::App(err)
//     }
// }

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

impl<E: Debug + Display> From<http2::Error> for HttpDispatchError<E> {
    fn from(err: http2::Error) -> Self {
        HttpDispatchError::Http2(err)
    }
}

pub(crate) struct ServerError(Version, StatusCode);

impl ServerError {
    pub fn err(ver: Version, status: StatusCode) -> Box<HttpHandlerTask> {
        Box::new(ServerError(ver, status))
    }
}

impl HttpHandlerTask for ServerError {
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
        {
            let bytes = io.buffer();
            // Buffer should have sufficient capacity for status line
            // and extra space
            bytes.reserve(helpers::STATUS_LINE_BUF_SIZE + 1);
            helpers::write_status_line(self.0, self.1.as_u16(), bytes);
        }
        // Convert Status Code to Reason.
        let reason = self.1.canonical_reason().unwrap_or("");
        io.buffer().extend_from_slice(reason.as_bytes());
        // No response body.
        io.buffer().extend_from_slice(b"\r\ncontent-length: 0\r\n");
        // date header
        io.set_date();
        Ok(Async::Ready(true))
    }
}

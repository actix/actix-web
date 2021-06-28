//! Error and Result module

use std::{error::Error as StdError, fmt, io, str::Utf8Error, string::FromUtf8Error};

use derive_more::{Display, Error, From};
use http::{uri::InvalidUri, StatusCode};

use crate::{
    body::{AnyBody, Body},
    ws, Response,
};

pub use http::Error as HttpError;

pub struct Error {
    inner: Box<ErrorInner>,
}

pub(crate) struct ErrorInner {
    #[allow(dead_code)]
    kind: Kind,
    cause: Option<Box<dyn StdError>>,
}

impl Error {
    fn new(kind: Kind) -> Self {
        Self {
            inner: Box::new(ErrorInner { kind, cause: None }),
        }
    }

    pub(crate) fn new_http() -> Self {
        Self::new(Kind::Http)
    }

    pub(crate) fn new_parse() -> Self {
        Self::new(Kind::Parse)
    }

    pub(crate) fn new_payload() -> Self {
        Self::new(Kind::Payload)
    }

    pub(crate) fn new_body() -> Self {
        Self::new(Kind::Body)
    }

    pub(crate) fn new_send_response() -> Self {
        Self::new(Kind::SendResponse)
    }

    // TODO: remove allow
    #[allow(dead_code)]
    pub(crate) fn new_io() -> Self {
        Self::new(Kind::Io)
    }

    pub(crate) fn new_encoder() -> Self {
        Self::new(Kind::Encoder)
    }

    pub(crate) fn new_ws() -> Self {
        Self::new(Kind::Ws)
    }

    pub(crate) fn with_cause(mut self, cause: impl Into<Box<dyn StdError>>) -> Self {
        self.inner.cause = Some(cause.into());
        self
    }
}

impl From<Error> for Response<AnyBody> {
    fn from(err: Error) -> Self {
        let status_code = match err.inner.kind {
            Kind::Parse => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        Response::new(status_code).set_body(Body::from(err.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum Kind {
    #[display(fmt = "error processing HTTP")]
    Http,

    #[display(fmt = "error parsing HTTP message")]
    Parse,

    #[display(fmt = "request payload read error")]
    Payload,

    #[display(fmt = "response body write error")]
    Body,

    #[display(fmt = "send response error")]
    SendResponse,

    #[display(fmt = "error in WebSocket process")]
    Ws,

    #[display(fmt = "connection error")]
    Io,

    #[display(fmt = "encoder error")]
    Encoder,
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: more detail
        f.write_str("actix_http::Error")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.cause.as_ref() {
            Some(err) => write!(f, "{}: {}", &self.inner.kind, err),
            None => write!(f, "{}", &self.inner.kind),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner.cause.as_ref().map(Box::as_ref)
    }
}

impl From<std::convert::Infallible> for Error {
    fn from(err: std::convert::Infallible) -> Self {
        match err {}
    }
}

impl From<ws::ProtocolError> for Error {
    fn from(err: ws::ProtocolError) -> Self {
        Self::new_ws().with_cause(err)
    }
}

impl From<HttpError> for Error {
    fn from(err: HttpError) -> Self {
        Self::new_http().with_cause(err)
    }
}

impl From<ws::HandshakeError> for Error {
    fn from(err: ws::HandshakeError) -> Self {
        Self::new_ws().with_cause(err)
    }
}

/// A set of errors that can occur during parsing HTTP streams.
#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum ParseError {
    /// An invalid `Method`, such as `GE.T`.
    #[display(fmt = "Invalid Method specified")]
    Method,

    /// An invalid `Uri`, such as `exam ple.domain`.
    #[display(fmt = "Uri error: {}", _0)]
    Uri(InvalidUri),

    /// An invalid `HttpVersion`, such as `HTP/1.1`
    #[display(fmt = "Invalid HTTP version specified")]
    Version,

    /// An invalid `Header`.
    #[display(fmt = "Invalid Header provided")]
    Header,

    /// A message head is too large to be reasonable.
    #[display(fmt = "Message head is too large")]
    TooLarge,

    /// A message reached EOF, but is not complete.
    #[display(fmt = "Message is incomplete")]
    Incomplete,

    /// An invalid `Status`, such as `1337 ELITE`.
    #[display(fmt = "Invalid Status provided")]
    Status,

    /// A timeout occurred waiting for an IO event.
    #[allow(dead_code)]
    #[display(fmt = "Timeout")]
    Timeout,

    /// An `io::Error` that occurred while trying to read or write to a network stream.
    #[display(fmt = "IO error: {}", _0)]
    Io(io::Error),

    /// Parsing a field as string failed
    #[display(fmt = "UTF8 error: {}", _0)]
    Utf8(Utf8Error),
}

impl From<io::Error> for ParseError {
    fn from(err: io::Error) -> ParseError {
        ParseError::Io(err)
    }
}

impl From<InvalidUri> for ParseError {
    fn from(err: InvalidUri) -> ParseError {
        ParseError::Uri(err)
    }
}

impl From<Utf8Error> for ParseError {
    fn from(err: Utf8Error) -> ParseError {
        ParseError::Utf8(err)
    }
}

impl From<FromUtf8Error> for ParseError {
    fn from(err: FromUtf8Error) -> ParseError {
        ParseError::Utf8(err.utf8_error())
    }
}

impl From<httparse::Error> for ParseError {
    fn from(err: httparse::Error) -> ParseError {
        match err {
            httparse::Error::HeaderName
            | httparse::Error::HeaderValue
            | httparse::Error::NewLine
            | httparse::Error::Token => ParseError::Header,
            httparse::Error::Status => ParseError::Status,
            httparse::Error::TooManyHeaders => ParseError::TooLarge,
            httparse::Error::Version => ParseError::Version,
        }
    }
}

impl From<ParseError> for Error {
    fn from(err: ParseError) -> Self {
        Self::new_parse().with_cause(err)
    }
}

impl From<ParseError> for Response<AnyBody> {
    fn from(err: ParseError) -> Self {
        Error::from(err).into()
    }
}

/// A set of errors that can occur running blocking tasks in thread pool.
#[derive(Debug, Display, Error)]
#[display(fmt = "Blocking thread pool is gone")]
pub struct BlockingError;

/// A set of errors that can occur during payload parsing.
#[derive(Debug, Display)]
#[non_exhaustive]
pub enum PayloadError {
    /// A payload reached EOF, but is not complete.
    #[display(
        fmt = "A payload reached EOF, but is not complete. Inner error: {:?}",
        _0
    )]
    Incomplete(Option<io::Error>),

    /// Content encoding stream corruption.
    #[display(fmt = "Can not decode content-encoding.")]
    EncodingCorrupted,

    /// Payload reached size limit.
    #[display(fmt = "Payload reached size limit.")]
    Overflow,

    /// Payload length is unknown.
    #[display(fmt = "Payload length is unknown.")]
    UnknownLength,

    /// HTTP/2 payload error.
    #[display(fmt = "{}", _0)]
    Http2Payload(h2::Error),

    /// Generic I/O error.
    #[display(fmt = "{}", _0)]
    Io(io::Error),
}

impl std::error::Error for PayloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PayloadError::Incomplete(None) => None,
            PayloadError::Incomplete(Some(err)) => Some(err as &dyn std::error::Error),
            PayloadError::EncodingCorrupted => None,
            PayloadError::Overflow => None,
            PayloadError::UnknownLength => None,
            PayloadError::Http2Payload(err) => Some(err as &dyn std::error::Error),
            PayloadError::Io(err) => Some(err as &dyn std::error::Error),
        }
    }
}

impl From<h2::Error> for PayloadError {
    fn from(err: h2::Error) -> Self {
        PayloadError::Http2Payload(err)
    }
}

impl From<Option<io::Error>> for PayloadError {
    fn from(err: Option<io::Error>) -> Self {
        PayloadError::Incomplete(err)
    }
}

impl From<io::Error> for PayloadError {
    fn from(err: io::Error) -> Self {
        PayloadError::Incomplete(Some(err))
    }
}

impl From<BlockingError> for PayloadError {
    fn from(_: BlockingError) -> Self {
        PayloadError::Io(io::Error::new(
            io::ErrorKind::Other,
            "Operation is canceled",
        ))
    }
}

impl From<PayloadError> for Error {
    fn from(err: PayloadError) -> Self {
        Self::new_payload().with_cause(err)
    }
}

/// A set of errors that can occur during dispatching HTTP requests.
#[derive(Debug, Display, Error, From)]
#[non_exhaustive]
pub enum DispatchError {
    /// Service error
    // FIXME: display and error type
    #[display(fmt = "Service Error")]
    Service(#[error(not(source))] Response<AnyBody>),

    /// Body error
    // FIXME: display and error type
    #[display(fmt = "Body Error")]
    Body(#[error(not(source))] Box<dyn StdError>),

    /// Upgrade service error
    Upgrade,

    /// An `io::Error` that occurred while trying to read or write to a network stream.
    #[display(fmt = "IO error: {}", _0)]
    Io(io::Error),

    /// Http request parse error.
    #[display(fmt = "Parse error: {}", _0)]
    Parse(ParseError),

    /// Http/2 error
    #[display(fmt = "{}", _0)]
    H2(h2::Error),

    /// The first request did not complete within the specified timeout.
    #[display(fmt = "The first request did not complete within the specified timeout")]
    SlowRequestTimeout,

    /// Disconnect timeout. Makes sense for ssl streams.
    #[display(fmt = "Connection shutdown timeout")]
    DisconnectTimeout,

    /// Payload is not consumed
    #[display(fmt = "Task is completed but request's payload is not consumed")]
    PayloadIsNotConsumed,

    /// Malformed request
    #[display(fmt = "Malformed request")]
    MalformedRequest,

    /// Internal error
    #[display(fmt = "Internal error")]
    InternalError,

    /// Unknown error
    #[display(fmt = "Unknown error")]
    Unknown,
}

/// A set of error that can occur during parsing content type.
#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum ContentTypeError {
    /// Can not parse content type
    #[display(fmt = "Can not parse content type")]
    ParseError,

    /// Unknown content encoding
    #[display(fmt = "Unknown content encoding")]
    UnknownEncoding,
}

#[cfg(test)]
mod content_type_test_impls {
    use super::*;

    impl std::cmp::PartialEq for ContentTypeError {
        fn eq(&self, other: &Self) -> bool {
            match self {
                Self::ParseError => matches!(other, ContentTypeError::ParseError),
                Self::UnknownEncoding => {
                    matches!(other, ContentTypeError::UnknownEncoding)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Error as HttpError, StatusCode};
    use std::io;

    #[test]
    fn test_into_response() {
        let resp: Response<AnyBody> = ParseError::Incomplete.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let err: HttpError = StatusCode::from_u16(10000).err().unwrap().into();
        let resp: Response<AnyBody> = Error::new_http().with_cause(err).into();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_as_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let err: Error = ParseError::Io(orig).into();
        assert_eq!(
            format!("{}", err),
            "error parsing HTTP message: IO error: other"
        );
    }

    #[test]
    fn test_error_display() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let err = Error::new_io().with_cause(orig);
        assert_eq!("connection error: other", err.to_string());
    }

    #[test]
    fn test_error_http_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let err = Error::new_io().with_cause(orig);
        let resp: Response<AnyBody> = err.into();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_payload_error() {
        let err: PayloadError =
            io::Error::new(io::ErrorKind::Other, "ParseError").into();
        assert!(err.to_string().contains("ParseError"));

        let err = PayloadError::Incomplete(None);
        assert_eq!(
            err.to_string(),
            "A payload reached EOF, but is not complete. Inner error: None"
        );
    }

    macro_rules! from {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                err @ $error => {
                    assert!(err.to_string().len() >= 5);
                }
                err => unreachable!("{:?}", err),
            }
        };
    }

    macro_rules! from_and_cause {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                e @ $error => {
                    let desc = format!("{}", e);
                    assert_eq!(desc, format!("IO error: {}", $from));
                }
                _ => unreachable!("{:?}", $from),
            }
        };
    }

    #[test]
    fn test_from() {
        from_and_cause!(io::Error::new(io::ErrorKind::Other, "other") => ParseError::Io(..));
        from!(httparse::Error::HeaderName => ParseError::Header);
        from!(httparse::Error::HeaderName => ParseError::Header);
        from!(httparse::Error::HeaderValue => ParseError::Header);
        from!(httparse::Error::NewLine => ParseError::Header);
        from!(httparse::Error::Status => ParseError::Status);
        from!(httparse::Error::Token => ParseError::Header);
        from!(httparse::Error::TooManyHeaders => ParseError::TooLarge);
        from!(httparse::Error::Version => ParseError::Version);
    }
}

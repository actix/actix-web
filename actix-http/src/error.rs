//! Error and Result module

use std::{error::Error as StdError, fmt, io, str::Utf8Error, string::FromUtf8Error};

use derive_more::{Display, Error, From};
pub use http::{status::InvalidStatusCode, Error as HttpError};
use http::{uri::InvalidUri, StatusCode};

use crate::{body::BoxBody, Response};

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

    pub(crate) fn with_cause(mut self, cause: impl Into<Box<dyn StdError>>) -> Self {
        self.inner.cause = Some(cause.into());
        self
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

    #[allow(unused)] // available for future use
    pub(crate) fn new_io() -> Self {
        Self::new(Kind::Io)
    }

    #[allow(unused)] // used in encoder behind feature flag so ignore unused warning
    pub(crate) fn new_encoder() -> Self {
        Self::new(Kind::Encoder)
    }

    #[allow(unused)] // used with `ws` feature flag
    pub(crate) fn new_ws() -> Self {
        Self::new(Kind::Ws)
    }
}

impl From<Error> for Response<BoxBody> {
    fn from(err: Error) -> Self {
        // TODO: more appropriate error status codes, usage assessment needed
        let status_code = match err.inner.kind {
            Kind::Parse => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        Response::new(status_code).set_body(BoxBody::new(err.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub(crate) enum Kind {
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
        f.debug_struct("actix_http::Error")
            .field("kind", &self.inner.kind)
            .field("cause", &self.inner.cause)
            .finish()
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

impl From<HttpError> for Error {
    fn from(err: HttpError) -> Self {
        Self::new_http().with_cause(err)
    }
}

#[cfg(feature = "ws")]
impl From<crate::ws::HandshakeError> for Error {
    fn from(err: crate::ws::HandshakeError) -> Self {
        Self::new_ws().with_cause(err)
    }
}

#[cfg(feature = "ws")]
impl From<crate::ws::ProtocolError> for Error {
    fn from(err: crate::ws::ProtocolError) -> Self {
        Self::new_ws().with_cause(err)
    }
}

/// A set of errors that can occur during parsing HTTP streams.
#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum ParseError {
    /// An invalid `Method`, such as `GE.T`.
    #[display(fmt = "invalid method specified")]
    Method,

    /// An invalid `Uri`, such as `exam ple.domain`.
    #[display(fmt = "URI error: {}", _0)]
    Uri(InvalidUri),

    /// An invalid `HttpVersion`, such as `HTP/1.1`
    #[display(fmt = "invalid HTTP version specified")]
    Version,

    /// An invalid `Header`.
    #[display(fmt = "invalid Header provided")]
    Header,

    /// A message head is too large to be reasonable.
    #[display(fmt = "message head is too large")]
    TooLarge,

    /// A message reached EOF, but is not complete.
    #[display(fmt = "message is incomplete")]
    Incomplete,

    /// An invalid `Status`, such as `1337 ELITE`.
    #[display(fmt = "invalid status provided")]
    Status,

    /// A timeout occurred waiting for an IO event.
    #[allow(dead_code)]
    #[display(fmt = "timeout")]
    Timeout,

    /// An I/O error that occurred while trying to read or write to a network stream.
    #[display(fmt = "I/O error: {}", _0)]
    Io(io::Error),

    /// Parsing a field as string failed.
    #[display(fmt = "UTF-8 error: {}", _0)]
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

impl From<ParseError> for Response<BoxBody> {
    fn from(err: ParseError) -> Self {
        Error::from(err).into()
    }
}

/// A set of errors that can occur during payload parsing.
#[derive(Debug, Display)]
#[non_exhaustive]
pub enum PayloadError {
    /// A payload reached EOF, but is not complete.
    #[display(fmt = "payload reached EOF before completing: {:?}", _0)]
    Incomplete(Option<io::Error>),

    /// Content encoding stream corruption.
    #[display(fmt = "can not decode content-encoding")]
    EncodingCorrupted,

    /// Payload reached size limit.
    #[display(fmt = "payload reached size limit")]
    Overflow,

    /// Payload length is unknown.
    #[display(fmt = "payload length is unknown")]
    UnknownLength,

    /// HTTP/2 payload error.
    #[cfg(feature = "http2")]
    #[display(fmt = "{}", _0)]
    Http2Payload(::h2::Error),

    /// Generic I/O error.
    #[display(fmt = "{}", _0)]
    Io(io::Error),
}

impl std::error::Error for PayloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PayloadError::Incomplete(None) => None,
            PayloadError::Incomplete(Some(err)) => Some(err),
            PayloadError::EncodingCorrupted => None,
            PayloadError::Overflow => None,
            PayloadError::UnknownLength => None,
            #[cfg(feature = "http2")]
            PayloadError::Http2Payload(err) => Some(err),
            PayloadError::Io(err) => Some(err),
        }
    }
}

#[cfg(feature = "http2")]
impl From<::h2::Error> for PayloadError {
    fn from(err: ::h2::Error) -> Self {
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

impl From<PayloadError> for Error {
    fn from(err: PayloadError) -> Self {
        Self::new_payload().with_cause(err)
    }
}

/// A set of errors that can occur during dispatching HTTP requests.
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum DispatchError {
    /// Service error.
    #[display(fmt = "service error")]
    Service(Response<BoxBody>),

    /// Body streaming error.
    #[display(fmt = "body error: {}", _0)]
    Body(Box<dyn StdError>),

    /// Upgrade service error.
    #[display(fmt = "upgrade error")]
    Upgrade,

    /// An `io::Error` that occurred while trying to read or write to a network stream.
    #[display(fmt = "I/O error: {}", _0)]
    Io(io::Error),

    /// Request parse error.
    #[display(fmt = "request parse error: {}", _0)]
    Parse(ParseError),

    /// HTTP/2 error.
    #[display(fmt = "{}", _0)]
    #[cfg(feature = "http2")]
    H2(h2::Error),

    /// The first request did not complete within the specified timeout.
    #[display(fmt = "request did not complete within the specified timeout")]
    SlowRequestTimeout,

    /// Disconnect timeout. Makes sense for TLS streams.
    #[display(fmt = "connection shutdown timeout")]
    DisconnectTimeout,

    /// Handler dropped payload before reading EOF.
    #[display(fmt = "handler dropped payload before reading EOF")]
    HandlerDroppedPayload,

    /// Internal error.
    #[display(fmt = "internal error")]
    InternalError,
}

impl StdError for DispatchError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            DispatchError::Service(_res) => None,
            DispatchError::Body(err) => Some(&**err),
            DispatchError::Io(err) => Some(err),
            DispatchError::Parse(err) => Some(err),

            #[cfg(feature = "http2")]
            DispatchError::H2(err) => Some(err),

            _ => None,
        }
    }
}

/// A set of error that can occur during parsing content type.
#[derive(Debug, Display, Error)]
#[cfg_attr(test, derive(PartialEq, Eq))]
#[non_exhaustive]
pub enum ContentTypeError {
    /// Can not parse content type.
    #[display(fmt = "could not parse content type")]
    ParseError,

    /// Unknown content encoding.
    #[display(fmt = "unknown content encoding")]
    UnknownEncoding,
}

#[cfg(test)]
mod tests {
    use http::Error as HttpError;

    use super::*;

    #[test]
    fn test_into_response() {
        let resp: Response<BoxBody> = ParseError::Incomplete.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let err: HttpError = StatusCode::from_u16(10000).err().unwrap().into();
        let resp: Response<BoxBody> = Error::new_http().with_cause(err).into();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_as_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let err: Error = ParseError::Io(orig).into();
        assert_eq!(
            format!("{}", err),
            "error parsing HTTP message: I/O error: other"
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
        let resp: Response<BoxBody> = err.into();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_payload_error() {
        let err: PayloadError = io::Error::new(io::ErrorKind::Other, "ParseError").into();
        assert!(err.to_string().contains("ParseError"));

        let err = PayloadError::Incomplete(None);
        assert_eq!(
            err.to_string(),
            "payload reached EOF before completing: None"
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
                    assert_eq!(desc, format!("I/O error: {}", $from));
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

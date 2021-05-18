//! Error and Result module

use std::{
    error::Error as StdError,
    fmt,
    io::{self, Write as _},
    str::Utf8Error,
    string::FromUtf8Error,
};

use bytes::BytesMut;
use derive_more::{Display, Error, From};
use http::{header, uri::InvalidUri, StatusCode};
use serde::de::value::Error as DeError;

use crate::{
    body::{AnyBody, Body},
    helpers::Writer,
    Response,
};

pub use http::Error as HttpError;

/// General purpose actix web error.
///
/// An actix web error is used to carry errors from `std::error`
/// through actix in a convenient way.  It can be created through
/// converting errors with `into()`.
///
/// Whenever it is created from an external object a response error is created
/// for it that can be used to create an HTTP response from it this means that
/// if you have access to an actix `Error` you can always get a
/// `ResponseError` reference from it.
pub struct Error {
    cause: Box<dyn ResponseError>,
}

impl Error {
    /// Returns the reference to the underlying `ResponseError`.
    pub fn as_response_error(&self) -> &dyn ResponseError {
        self.cause.as_ref()
    }

    /// Similar to `as_response_error` but downcasts.
    pub fn as_error<T: ResponseError + 'static>(&self) -> Option<&T> {
        <dyn ResponseError>::downcast_ref(self.cause.as_ref())
    }
}

/// Errors that can generate responses.
// TODO: add std::error::Error bound when replacement for Box<dyn Error> is found
pub trait ResponseError: fmt::Debug + fmt::Display {
    /// Returns appropriate status code for error.
    ///
    /// A 500 Internal Server Error is used by default. If [error_response](Self::error_response) is
    /// also implemented and does not call `self.status_code()`, then this will not be used.
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    /// Creates full response for error.
    ///
    /// By default, the generated response uses a 500 Internal Server Error status code, a
    /// `Content-Type` of `text/plain`, and the body is set to `Self`'s `Display` impl.
    fn error_response(&self) -> Response<Body> {
        let mut resp = Response::new(self.status_code());
        let mut buf = BytesMut::new();
        let _ = write!(Writer(&mut buf), "{}", self);
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        resp.set_body(Body::from(buf))
    }

    downcast_get_type_id!();
}

downcast!(ResponseError);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.cause)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl From<()> for Error {
    fn from(_: ()) -> Self {
        Error::from(UnitError)
    }
}

impl From<std::convert::Infallible> for Error {
    fn from(val: std::convert::Infallible) -> Self {
        match val {}
    }
}

/// Convert `Error` to a `Response` instance
impl From<Error> for Response<Body> {
    fn from(err: Error) -> Self {
        Response::from_error(err)
    }
}

/// `Error` for any error that implements `ResponseError`
impl<T: ResponseError + 'static> From<T> for Error {
    fn from(err: T) -> Error {
        Error {
            cause: Box::new(err),
        }
    }
}

#[derive(Debug, Display, Error)]
#[display(fmt = "Unknown Error")]
pub(crate) struct UnitError;

impl ResponseError for Box<dyn StdError + 'static> {}

/// Returns [`StatusCode::INTERNAL_SERVER_ERROR`] for [`UnitError`].
impl ResponseError for UnitError {}

/// Returns [`StatusCode::INTERNAL_SERVER_ERROR`] for [`actix_tls::accept::openssl::SslError`].
#[cfg(feature = "openssl")]
impl ResponseError for actix_tls::accept::openssl::SslError {}

/// Returns [`StatusCode::BAD_REQUEST`] for [`DeError`].
impl ResponseError for DeError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// Returns [`StatusCode::BAD_REQUEST`] for [`Utf8Error`].
impl ResponseError for Utf8Error {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// Returns [`StatusCode::INTERNAL_SERVER_ERROR`] for [`HttpError`].
impl ResponseError for HttpError {}

/// Inspects the underlying [`io::ErrorKind`] and returns an appropriate status code.
///
/// If the error is [`io::ErrorKind::NotFound`], [`StatusCode::NOT_FOUND`] is returned. If the
/// error is [`io::ErrorKind::PermissionDenied`], [`StatusCode::FORBIDDEN`] is returned. Otherwise,
/// [`StatusCode::INTERNAL_SERVER_ERROR`] is returned.
impl ResponseError for io::Error {
    fn status_code(&self) -> StatusCode {
        match self.kind() {
            io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
            io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Returns [`StatusCode::BAD_REQUEST`] for [`header::InvalidHeaderValue`].
impl ResponseError for header::InvalidHeaderValue {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
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

/// Return `BadRequest` for `ParseError`
impl ResponseError for ParseError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
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

/// A set of errors that can occur running blocking tasks in thread pool.
#[derive(Debug, Display, Error)]
#[display(fmt = "Blocking thread pool is gone")]
pub struct BlockingError;

/// `InternalServerError` for `BlockingError`
impl ResponseError for BlockingError {}

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

/// `PayloadError` returns two possible results:
///
/// - `Overflow` returns `PayloadTooLarge`
/// - Other errors returns `BadRequest`
impl ResponseError for PayloadError {
    fn status_code(&self) -> StatusCode {
        match *self {
            PayloadError::Overflow => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        }
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

impl ResponseError for ContentTypeError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Error as HttpError, StatusCode};
    use std::io;

    #[test]
    fn test_into_response() {
        let resp: Response<Body> = ParseError::Incomplete.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let err: HttpError = StatusCode::from_u16(10000).err().unwrap().into();
        let resp: Response<Body> = err.error_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_as_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let e: Error = ParseError::Io(orig).into();
        assert_eq!(format!("{}", e.as_response_error()), "IO error: other");
    }

    #[test]
    fn test_error_cause() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.to_string();
        let e = Error::from(orig);
        assert_eq!(format!("{}", e.as_response_error()), desc);
    }

    #[test]
    fn test_error_display() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.to_string();
        let e = Error::from(orig);
        assert_eq!(format!("{}", e), desc);
    }

    #[test]
    fn test_error_http_response() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let e = Error::from(orig);
        let resp: Response<Body> = e.into();
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

    #[test]
    fn test_error_casting() {
        let err = PayloadError::Overflow;
        let resp_err: &dyn ResponseError = &err;
        let err = resp_err.downcast_ref::<PayloadError>().unwrap();
        assert_eq!(err.to_string(), "Payload reached size limit.");
        let not_err = resp_err.downcast_ref::<ContentTypeError>();
        assert!(not_err.is_none());
    }
}

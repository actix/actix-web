//! Error and Result module.
use std::error::Error as StdError;
use std::fmt;
use std::io::Error as IoError;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

use cookie;
use httparse;
use http::{StatusCode, Error as HttpError};

use HttpRangeParseError;
use httpresponse::{Body, HttpResponse};


/// A set of errors that can occur during parsing HTTP streams.
#[derive(Debug)]
pub enum ParseError {
    /// An invalid `Method`, such as `GE,T`.
    Method,
    /// An invalid `Uri`, such as `exam ple.domain`.
    Uri,
    /// An invalid `HttpVersion`, such as `HTP/1.1`
    Version,
    /// An invalid `Header`.
    Header,
    /// A message head is too large to be reasonable.
    TooLarge,
    /// A message reached EOF, but is not complete.
    Incomplete,
    /// An invalid `Status`, such as `1337 ELITE`.
    Status,
    /// A timeout occurred waiting for an IO event.
    #[allow(dead_code)]
    Timeout,
    /// An `io::Error` that occurred while trying to read or write to a network stream.
    Io(IoError),
    /// Parsing a field as string failed
    Utf8(Utf8Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ParseError::Io(ref e) => fmt::Display::fmt(e, f),
            ParseError::Utf8(ref e) => fmt::Display::fmt(e, f),
            ref e => f.write_str(e.description()),
        }
    }
}

impl StdError for ParseError {
    fn description(&self) -> &str {
        match *self {
            ParseError::Method => "Invalid Method specified",
            ParseError::Version => "Invalid HTTP version specified",
            ParseError::Header => "Invalid Header provided",
            ParseError::TooLarge => "Message head is too large",
            ParseError::Status => "Invalid Status provided",
            ParseError::Incomplete => "Message is incomplete",
            ParseError::Timeout => "Timeout",
            ParseError::Uri => "Uri error",
            ParseError::Io(ref e) => e.description(),
            ParseError::Utf8(ref e) => e.description(),
        }
    }

    fn cause(&self) -> Option<&StdError> {
        match *self {
            ParseError::Io(ref error) => Some(error),
            ParseError::Utf8(ref error) => Some(error),
            _ => None,
        }
    }
}

impl From<IoError> for ParseError {
    fn from(err: IoError) -> ParseError {
        ParseError::Io(err)
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
            httparse::Error::HeaderName |
            httparse::Error::HeaderValue |
            httparse::Error::NewLine |
            httparse::Error::Token => ParseError::Header,
            httparse::Error::Status => ParseError::Status,
            httparse::Error::TooManyHeaders => ParseError::TooLarge,
            httparse::Error::Version => ParseError::Version,
        }
    }
}

/// Return `BadRequest` for `ParseError`
impl From<ParseError> for HttpResponse {
    fn from(err: ParseError) -> Self {
        HttpResponse::new(StatusCode::BAD_REQUEST,
                          Body::Binary(err.description().into()))
    }
}

/// Return `InternalServerError` for `HttpError`,
/// Response generation can return `HttpError`, so it is internal error
impl From<HttpError> for HttpResponse {
    fn from(err: HttpError) -> Self {
        HttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR,
                          Body::Binary(err.description().into()))
    }
}

/// Return `BadRequest` for `cookie::ParseError`
impl From<cookie::ParseError> for HttpResponse {
    fn from(err: cookie::ParseError) -> Self {
        HttpResponse::new(StatusCode::BAD_REQUEST,
                          Body::Binary(err.description().into()))
    }
}

/// Return `BadRequest` for `HttpRangeParseError`
impl From<HttpRangeParseError> for HttpResponse {
    fn from(_: HttpRangeParseError) -> Self {
        HttpResponse::new(StatusCode::BAD_REQUEST,
                          Body::Binary("Invalid Range header provided".into()))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as StdError;
    use std::io;
    use httparse;
    use http::StatusCode;
    use cookie::ParseError as CookieParseError;
    use super::{ParseError, HttpResponse, HttpRangeParseError};

    #[test]
    fn test_into_response() {
        let resp: HttpResponse = ParseError::Incomplete.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp: HttpResponse = HttpRangeParseError::InvalidRange.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp: HttpResponse = CookieParseError::EmptyName.into();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

    #[test]
    fn test_cause() {
        let orig = io::Error::new(io::ErrorKind::Other, "other");
        let desc = orig.description().to_owned();
        let e = ParseError::Io(orig);
        assert_eq!(e.cause().unwrap().description(), desc);
    }

    macro_rules! from {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                e @ $error => {
                    assert!(e.description().len() >= 5);
                } ,
                e => panic!("{:?}", e)
            }
        }
    }

    macro_rules! from_and_cause {
        ($from:expr => $error:pat) => {
            match ParseError::from($from) {
                e @ $error => {
                    let desc = e.cause().unwrap().description();
                    assert_eq!(desc, $from.description().to_owned());
                    assert_eq!(desc, e.description());
                },
                _ => panic!("{:?}", $from)
            }
        }
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

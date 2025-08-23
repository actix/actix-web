//! Error and Result module

// This is meant to be a glob import of the whole error module except for `Error`. Rustdoc can't yet
// correctly resolve the conflicting `Error` type defined in this module, so these re-exports are
// expanded manually.
//
// See <https://github.com/rust-lang/rust/issues/83375>
pub use actix_http::error::{ContentTypeError, DispatchError, HttpError, ParseError, PayloadError};
use derive_more::{Display, Error, From};
use serde_json::error::Error as JsonError;
use serde_urlencoded::{de::Error as FormDeError, ser::Error as FormError};
use url::ParseError as UrlParseError;

use crate::http::StatusCode;

#[allow(clippy::module_inception)]
mod error;
mod internal;
mod macros;
mod response_error;

pub(crate) use self::macros::{downcast_dyn, downcast_get_type_id};
pub use self::{error::Error, internal::*, response_error::ResponseError};

/// A convenience [`Result`](std::result::Result) for Actix Web operations.
///
/// This type alias is generally used to avoid writing out `actix_http::Error` directly.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error representing a problem running a blocking task on a thread pool.
#[derive(Debug, Display, Error)]
#[display("Blocking thread pool is shut down unexpectedly")]
#[non_exhaustive]
pub struct BlockingError;

impl ResponseError for crate::error::BlockingError {}

/// Errors which can occur when attempting to generate resource uri.
#[derive(Debug, PartialEq, Eq, Display, Error, From)]
#[non_exhaustive]
pub enum UrlGenerationError {
    /// Resource not found.
    #[display("Resource not found")]
    ResourceNotFound,

    /// Not all URL parameters covered.
    #[display("Not all URL parameters covered")]
    NotEnoughElements,

    /// URL parse error.
    #[display("{}", _0)]
    ParseError(UrlParseError),
}

impl ResponseError for UrlGenerationError {}

/// A set of errors that can occur during parsing urlencoded payloads
#[derive(Debug, Display, Error, From)]
#[non_exhaustive]
pub enum UrlencodedError {
    /// Can not decode chunked transfer encoding.
    #[display("Can not decode chunked transfer encoding.")]
    Chunked,

    /// Payload size is larger than allowed. (default limit: 256kB).
    #[display(
        "URL encoded payload is larger ({} bytes) than allowed (limit: {} bytes).",
        size,
        limit
    )]
    Overflow { size: usize, limit: usize },

    /// Payload size is now known.
    #[display("Payload size is now known.")]
    UnknownLength,

    /// Content type error.
    #[display("Content type error.")]
    ContentType,

    /// Parse error.
    #[display("Parse error: {}.", _0)]
    Parse(FormDeError),

    /// Encoding error.
    #[display("Encoding error.")]
    Encoding,

    /// Serialize error.
    #[display("Serialize error: {}.", _0)]
    Serialize(FormError),

    /// Payload error.
    #[display("Error that occur during reading payload: {}.", _0)]
    Payload(PayloadError),
}

impl ResponseError for UrlencodedError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Overflow { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnknownLength => StatusCode::LENGTH_REQUIRED,
            Self::ContentType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Payload(err) => err.status_code(),
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

/// A set of errors that can occur during parsing json payloads
#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum JsonPayloadError {
    /// Payload size is bigger than allowed & content length header set. (default: 2MB)
    #[display(
        "JSON payload ({} bytes) is larger than allowed (limit: {} bytes).",
        length,
        limit
    )]
    OverflowKnownLength { length: usize, limit: usize },

    /// Payload size is bigger than allowed but no content length header set. (default: 2MB)
    #[display("JSON payload has exceeded limit ({} bytes).", limit)]
    Overflow { limit: usize },

    /// Content type error
    #[display("Content type error")]
    ContentType,

    /// Deserialize error
    #[display("Json deserialize error: {}", _0)]
    Deserialize(JsonError),

    /// Serialize error
    #[display("Json serialize error: {}", _0)]
    Serialize(JsonError),

    /// Payload error
    #[display("Error that occur during reading payload: {}", _0)]
    Payload(PayloadError),
}

impl From<PayloadError> for JsonPayloadError {
    fn from(err: PayloadError) -> Self {
        Self::Payload(err)
    }
}

impl ResponseError for JsonPayloadError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::OverflowKnownLength {
                length: _,
                limit: _,
            } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Overflow { limit: _ } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Serialize(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Payload(err) => err.status_code(),
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

/// A set of errors that can occur during parsing request paths
#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum PathError {
    /// Deserialize error
    #[display("Path deserialize error: {}", _0)]
    Deserialize(serde::de::value::Error),
}

/// Return `BadRequest` for `PathError`
impl ResponseError for PathError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// A set of errors that can occur during parsing query strings.
#[derive(Debug, Display, Error, From)]
#[non_exhaustive]
pub enum QueryPayloadError {
    /// Query deserialize error.
    #[display("Query deserialize error: {}", _0)]
    Deserialize(serde::de::value::Error),
}

impl ResponseError for QueryPayloadError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// Error type returned when reading body as lines.
#[derive(Debug, Display, Error, From)]
#[non_exhaustive]
pub enum ReadlinesError {
    #[display("Encoding error")]
    /// Payload size is bigger than allowed. (default: 256kB)
    EncodingError,

    /// Payload error.
    #[display("Error that occur during reading payload: {}", _0)]
    Payload(PayloadError),

    /// Line limit exceeded.
    #[display("Line limit exceeded")]
    LimitOverflow,

    /// ContentType error.
    #[display("Content-type error")]
    ContentTypeError(ContentTypeError),
}

impl ResponseError for ReadlinesError {
    fn status_code(&self) -> StatusCode {
        match *self {
            ReadlinesError::LimitOverflow => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoded_error() {
        let resp = UrlencodedError::Overflow { size: 0, limit: 0 }.error_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp = UrlencodedError::UnknownLength.error_response();
        assert_eq!(resp.status(), StatusCode::LENGTH_REQUIRED);
        let resp = UrlencodedError::ContentType.error_response();
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn test_json_payload_error() {
        let resp = JsonPayloadError::OverflowKnownLength {
            length: 0,
            limit: 0,
        }
        .error_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp = JsonPayloadError::Overflow { limit: 0 }.error_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp = JsonPayloadError::ContentType.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_query_payload_error() {
        let resp = QueryPayloadError::Deserialize(
            serde_urlencoded::from_str::<i32>("bad query").unwrap_err(),
        )
        .error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_readlines_error() {
        let resp = ReadlinesError::LimitOverflow.error_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp = ReadlinesError::EncodingError.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

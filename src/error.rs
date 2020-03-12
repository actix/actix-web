//! Error and Result module
pub use actix_http::error::*;
use serde_json::error::Error as JsonError;
use thiserror::Error;
use url::ParseError as UrlParseError;

use crate::http::StatusCode;
use crate::HttpResponse;

/// Errors which can occur when attempting to generate resource uri.
#[derive(Debug, PartialEq, Error)]
pub enum UrlGenerationError {
    /// Resource not found
    #[error("Resource not found")]
    ResourceNotFound,
    /// Not all path pattern covered
    #[error("Not all path pattern covered")]
    NotEnoughElements,
    /// URL parse error
    #[error(transparent)]
    ParseError(#[from] UrlParseError),
}

/// `InternalServerError` for `UrlGeneratorError`
impl ResponseError for UrlGenerationError {}

/// A set of errors that can occur during parsing urlencoded payloads
#[derive(Debug, Error)]
pub enum UrlencodedError {
    /// Can not decode chunked transfer encoding
    #[error("Can not decode chunked transfer encoding")]
    Chunked,
    /// Payload size is bigger than allowed. (default: 256kB)
    #[error("Urlencoded payload size is bigger ({size} bytes) than allowed (default: {limit} bytes)")]
    Overflow { size: usize, limit: usize },
    /// Payload size is now known
    #[error("Payload size is now known")]
    UnknownLength,
    /// Content type error
    #[error("Content type error")]
    ContentType,
    /// Parse error
    #[error("Parse error")]
    Parse,
    /// Payload error
    #[error("Error that occur during reading payload: {0}")]
    Payload(#[from] PayloadError),
}

/// Return `BadRequest` for `UrlencodedError`
impl ResponseError for UrlencodedError {
    fn status_code(&self) -> StatusCode {
        match *self {
            UrlencodedError::Overflow { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            UrlencodedError::UnknownLength => StatusCode::LENGTH_REQUIRED,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

/// A set of errors that can occur during parsing json payloads
#[derive(Debug, Error)]
pub enum JsonPayloadError {
    /// Payload size is bigger than allowed. (default: 32kB)
    #[error("Json payload size is bigger than allowed")]
    Overflow,
    /// Content type error
    #[error("Content type error")]
    ContentType,
    /// Deserialize error
    #[error("Json deserialize error: {0}")]
    Deserialize(#[from] JsonError),
    /// Payload error
    #[error("Error that occur during reading payload: {0}")]
    Payload(#[from] PayloadError),
}

/// Return `BadRequest` for `JsonPayloadError`
impl ResponseError for JsonPayloadError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            JsonPayloadError::Overflow => {
                HttpResponse::new(StatusCode::PAYLOAD_TOO_LARGE)
            }
            _ => HttpResponse::new(StatusCode::BAD_REQUEST),
        }
    }
}

/// A set of errors that can occur during parsing request paths
#[derive(Debug, Error)]
pub enum PathError {
    /// Deserialize error
    #[error("Path deserialize error: {0}")]
    Deserialize(#[from] serde::de::value::Error),
}

/// Return `BadRequest` for `PathError`
impl ResponseError for PathError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// A set of errors that can occur during parsing query strings
#[derive(Debug, Error)]
pub enum QueryPayloadError {
    /// Deserialize error
    #[error("Query deserialize error: {0}")]
    Deserialize(#[from] serde::de::value::Error),
}

/// Return `BadRequest` for `QueryPayloadError`
impl ResponseError for QueryPayloadError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// Error type returned when reading body as lines.
#[derive(Error, Debug)]
pub enum ReadlinesError {
    /// Error when decoding a line.
    #[error("Encoding error")]
    /// Payload size is bigger than allowed. (default: 256kB)
    EncodingError,
    /// Payload error.
    #[error("Error that occur during reading payload: {0}")]
    Payload(#[from] PayloadError),
    /// Line limit exceeded.
    #[error("Line limit exceeded")]
    LimitOverflow,
    /// ContentType error.
    #[error("Content-type error")]
    ContentTypeError(#[from] ContentTypeError),
}

/// Return `BadRequest` for `ReadlinesError`
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
        let resp: HttpResponse =
            UrlencodedError::Overflow { size: 0, limit: 0 }.error_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp: HttpResponse = UrlencodedError::UnknownLength.error_response();
        assert_eq!(resp.status(), StatusCode::LENGTH_REQUIRED);
        let resp: HttpResponse = UrlencodedError::ContentType.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_json_payload_error() {
        let resp: HttpResponse = JsonPayloadError::Overflow.error_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp: HttpResponse = JsonPayloadError::ContentType.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_query_payload_error() {
        let resp: HttpResponse = QueryPayloadError::Deserialize(
            serde_urlencoded::from_str::<i32>("bad query").unwrap_err(),
        )
        .error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_readlines_error() {
        let resp: HttpResponse = ReadlinesError::LimitOverflow.error_response();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let resp: HttpResponse = ReadlinesError::EncodingError.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

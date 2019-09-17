//! Error and Result module
pub use actix_http::error::*;
use derive_more::{Display, From};
use serde_json::error::Error as JsonError;
use url::ParseError as UrlParseError;

use crate::http::StatusCode;
use crate::HttpResponse;
use serde_urlencoded::de;

/// Errors which can occur when attempting to generate resource uri.
#[derive(Debug, PartialEq, Display, From)]
pub enum UrlGenerationError {
    /// Resource not found
    #[display(fmt = "Resource not found")]
    ResourceNotFound,
    /// Not all path pattern covered
    #[display(fmt = "Not all path pattern covered")]
    NotEnoughElements,
    /// URL parse error
    #[display(fmt = "{}", _0)]
    ParseError(UrlParseError),
}

/// `InternalServerError` for `UrlGeneratorError`
impl ResponseError for UrlGenerationError {}

/// A set of errors that can occur during parsing urlencoded payloads
#[derive(Debug, Display, From)]
pub enum UrlencodedError {
    /// Can not decode chunked transfer encoding
    #[display(fmt = "Can not decode chunked transfer encoding")]
    Chunked,
    /// Payload size is bigger than allowed. (default: 256kB)
    #[display(
        fmt = "Urlencoded payload size is bigger ({} bytes) than allowed (default: {} bytes)",
        size,
        limit
    )]
    Overflow { size: usize, limit: usize },
    /// Payload size is now known
    #[display(fmt = "Payload size is now known")]
    UnknownLength,
    /// Content type error
    #[display(fmt = "Content type error")]
    ContentType,
    /// Parse error
    #[display(fmt = "Parse error")]
    Parse,
    /// Payload error
    #[display(fmt = "Error that occur during reading payload: {}", _0)]
    Payload(PayloadError),
}

/// Return `BadRequest` for `UrlencodedError`
impl ResponseError for UrlencodedError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            UrlencodedError::Overflow { .. } => {
                HttpResponse::new(StatusCode::PAYLOAD_TOO_LARGE)
            }
            UrlencodedError::UnknownLength => {
                HttpResponse::new(StatusCode::LENGTH_REQUIRED)
            }
            _ => HttpResponse::new(StatusCode::BAD_REQUEST),
        }
    }
}

/// A set of errors that can occur during parsing json payloads
#[derive(Debug, Display, From)]
pub enum JsonPayloadError {
    /// Payload size is bigger than allowed. (default: 32kB)
    #[display(fmt = "Json payload size is bigger than allowed")]
    Overflow,
    /// Content type error
    #[display(fmt = "Content type error")]
    ContentType,
    /// Deserialize error
    #[display(fmt = "Json deserialize error: {}", _0)]
    Deserialize(JsonError),
    /// Payload error
    #[display(fmt = "Error that occur during reading payload: {}", _0)]
    Payload(PayloadError),
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
#[derive(Debug, Display, From)]
pub enum PathError {
    /// Deserialize error
    #[display(fmt = "Path deserialize error: {}", _0)]
    Deserialize(de::Error),
}

/// Return `BadRequest` for `PathError`
impl ResponseError for PathError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            PathError::Deserialize(_) => HttpResponse::new(StatusCode::BAD_REQUEST),
        }
    }
}

/// A set of errors that can occur during parsing query strings
#[derive(Debug, Display, From)]
pub enum QueryPayloadError {
    /// Deserialize error
    #[display(fmt = "Query deserialize error: {}", _0)]
    Deserialize(de::Error),
}

/// Return `BadRequest` for `QueryPayloadError`
impl ResponseError for QueryPayloadError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            QueryPayloadError::Deserialize(_) => {
                HttpResponse::new(StatusCode::BAD_REQUEST)
            }
        }
    }
}

/// Error type returned when reading body as lines.
#[derive(From, Display, Debug)]
pub enum ReadlinesError {
    /// Error when decoding a line.
    #[display(fmt = "Encoding error")]
    /// Payload size is bigger than allowed. (default: 256kB)
    EncodingError,
    /// Payload error.
    #[display(fmt = "Error that occur during reading payload: {}", _0)]
    Payload(PayloadError),
    /// Line limit exceeded.
    #[display(fmt = "Line limit exceeded")]
    LimitOverflow,
    /// ContentType error.
    #[display(fmt = "Content-type error")]
    ContentTypeError(ContentTypeError),
}

/// Return `BadRequest` for `ReadlinesError`
impl ResponseError for ReadlinesError {
    fn error_response(&self) -> HttpResponse {
        match *self {
            ReadlinesError::LimitOverflow => {
                HttpResponse::new(StatusCode::PAYLOAD_TOO_LARGE)
            }
            _ => HttpResponse::new(StatusCode::BAD_REQUEST),
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

//! Error and Result module
use std::fmt;

pub use actix_http::error::*;
use derive_more::{Display, From};
use serde_json::error::Error as JsonError;
use url::ParseError as UrlParseError;

use crate::http::StatusCode;
use crate::HttpResponse;

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

/// Blocking operation execution error
#[derive(Debug, Display)]
pub enum BlockingError<E: fmt::Debug> {
    #[display(fmt = "{:?}", _0)]
    Error(E),
    #[display(fmt = "Thread pool is gone")]
    Canceled,
}

impl<E: fmt::Debug> ResponseError for BlockingError<E> {}

impl<E: fmt::Debug> From<actix_rt::blocking::BlockingError<E>> for BlockingError<E> {
    fn from(err: actix_rt::blocking::BlockingError<E>) -> Self {
        match err {
            actix_rt::blocking::BlockingError::Error(e) => BlockingError::Error(e),
            actix_rt::blocking::BlockingError::Canceled => BlockingError::Canceled,
        }
    }
}

/// A set of errors that can occur during parsing urlencoded payloads
#[derive(Debug, Display, From)]
pub enum UrlencodedError {
    /// Can not decode chunked transfer encoding
    #[display(fmt = "Can not decode chunked transfer encoding")]
    Chunked,
    /// Payload size is bigger than allowed. (default: 256kB)
    #[display(fmt = "Urlencoded payload size is bigger than allowed. (default: 256kB)")]
    Overflow,
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
            UrlencodedError::Overflow => {
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
    /// Payload size is bigger than allowed. (default: 256kB)
    #[display(fmt = "Json payload size is bigger than allowed. (default: 256kB)")]
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

/// Return `BadRequest` for `UrlencodedError`
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

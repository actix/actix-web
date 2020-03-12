//! Error and Result module
use actix_web::error::{ParseError, PayloadError};
use actix_web::http::StatusCode;
use actix_web::ResponseError;
use thiserror::Error;

/// A set of errors that can occur during parsing multipart streams
#[derive(Debug, Error)]
pub enum MultipartError {
    /// Content-Type header is not found
    #[error("No Content-type header found")]
    NoContentType,
    /// Can not parse Content-Type header
    #[error("Can not parse Content-Type header")]
    ParseContentType,
    /// Multipart boundary is not found
    #[error("Multipart boundary is not found")]
    Boundary,
    /// Nested multipart is not supported
    #[error("Nested multipart is not supported")]
    Nested,
    /// Multipart stream is incomplete
    #[error("Multipart stream is incomplete")]
    Incomplete,
    /// Error during field parsing
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// Payload error
    #[error(transparent)]
    Payload(#[from] PayloadError),
    /// Not consumed
    #[error("Multipart stream is not consumed")]
    NotConsumed,
}

/// Return `BadRequest` for `MultipartError`
impl ResponseError for MultipartError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::HttpResponse;

    #[test]
    fn test_multipart_error() {
        let resp: HttpResponse = MultipartError::Boundary.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

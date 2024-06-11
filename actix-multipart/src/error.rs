//! Error and Result module

use actix_web::{
    error::{ParseError, PayloadError},
    http::StatusCode,
    ResponseError,
};
use derive_more::{Display, Error, From};

/// A set of errors that can occur during parsing multipart streams.
#[derive(Debug, Display, From, Error)]
#[non_exhaustive]
pub enum MultipartError {
    /// Content-Disposition header is not found or is not equal to "form-data".
    ///
    /// According to [RFC 7578 §4.2](https://datatracker.ietf.org/doc/html/rfc7578#section-4.2) a
    /// Content-Disposition header must always be present and equal to "form-data".
    #[display(fmt = "No Content-Disposition `form-data` header")]
    NoContentDisposition,

    /// Content-Type header is not found
    #[display(fmt = "No Content-Type header found")]
    NoContentType,

    /// Can not parse Content-Type header
    #[display(fmt = "Can not parse Content-Type header")]
    ParseContentType,

    /// Multipart boundary is not found
    #[display(fmt = "Multipart boundary is not found")]
    Boundary,

    /// Nested multipart is not supported
    #[display(fmt = "Nested multipart is not supported")]
    Nested,

    /// Multipart stream is incomplete
    #[display(fmt = "Multipart stream is incomplete")]
    Incomplete,

    /// Error during field parsing
    #[display(fmt = "{}", _0)]
    Parse(ParseError),

    /// Payload error
    #[display(fmt = "{}", _0)]
    Payload(PayloadError),

    /// Not consumed
    #[display(fmt = "Multipart stream is not consumed")]
    NotConsumed,

    /// An error from a field handler in a form
    #[display(
        fmt = "An error occurred processing field `{}`: {}",
        field_name,
        source
    )]
    Field {
        field_name: String,
        source: actix_web::Error,
    },

    /// Duplicate field
    #[display(fmt = "Duplicate field found for: `{}`", _0)]
    #[from(ignore)]
    DuplicateField(#[error(not(source))] String),

    /// Missing field
    #[display(fmt = "Field with name `{}` is required", _0)]
    #[from(ignore)]
    MissingField(#[error(not(source))] String),

    /// Unknown field
    #[display(fmt = "Unsupported field `{}`", _0)]
    #[from(ignore)]
    UnsupportedField(#[error(not(source))] String),
}

/// Return `BadRequest` for `MultipartError`
impl ResponseError for MultipartError {
    fn status_code(&self) -> StatusCode {
        match &self {
            MultipartError::Field { source, .. } => source.as_response_error().status_code(),
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multipart_error() {
        let resp = MultipartError::Boundary.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

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
pub enum Error {
    /// Could not find Content-Type header.
    #[display("Could not find Content-Type header")]
    ContentTypeMissing,

    /// Could not parse Content-Type header.
    #[display("Could not parse Content-Type header")]
    ContentTypeParse,

    /// Parsed Content-Type did not have "multipart" top-level media type.
    ///
    /// Also raised when extracting a [`MultipartForm`] from a request that does not have the
    /// "multipart/form-data" media type.
    ///
    /// [`MultipartForm`]: struct@crate::form::MultipartForm
    #[display("Parsed Content-Type did not have 'multipart' top-level media type")]
    ContentTypeIncompatible,

    /// Multipart boundary is not found.
    #[display("Multipart boundary is not found")]
    BoundaryMissing,

    /// Content-Disposition header was not found or not of disposition type "form-data" when parsing
    /// a "form-data" field.
    ///
    /// As per [RFC 7578 §4.2], a "multipart/form-data" field's Content-Disposition header must
    /// always be present and have a disposition type of "form-data".
    ///
    /// [RFC 7578 §4.2]: https://datatracker.ietf.org/doc/html/rfc7578#section-4.2
    #[display("Content-Disposition header was not found when parsing a \"form-data\" field")]
    ContentDispositionMissing,

    /// Content-Disposition name parameter was not found when parsing a "form-data" field.
    ///
    /// As per [RFC 7578 §4.2], a "multipart/form-data" field's Content-Disposition header must
    /// always include a "name" parameter.
    ///
    /// [RFC 7578 §4.2]: https://datatracker.ietf.org/doc/html/rfc7578#section-4.2
    #[display(
        "Content-Disposition 'name' parameter was not found when parsing a \"form-data\" field"
    )]
    ContentDispositionNameMissing,

    /// Nested multipart is not supported.
    #[display("Nested multipart is not supported")]
    Nested,

    /// Multipart stream is incomplete.
    #[display("Multipart stream is incomplete")]
    Incomplete,

    /// Field parsing failed.
    #[display("Error during field parsing")]
    Parse(ParseError),

    /// HTTP payload error.
    #[display("Payload error")]
    Payload(PayloadError),

    /// Stream is not consumed.
    #[display("Stream is not consumed")]
    NotConsumed,

    /// Form field handler raised error.
    #[display("An error occurred processing field \"{name}\"")]
    Field {
        name: String,
        source: actix_web::Error,
    },

    /// Duplicate field found (for structure that opted-in to denying duplicate fields).
    #[display("Duplicate field found \"{_0}\"")]
    #[from(ignore)]
    DuplicateField(#[error(not(source))] String),

    /// Required field is missing.
    #[display("Required field is missing \"{_0}\"")]
    #[from(ignore)]
    MissingField(#[error(not(source))] String),

    /// Unknown field (for structure that opted-in to denying unknown fields).
    #[display("Unknown field \"{_0}\"")]
    #[from(ignore)]
    UnknownField(#[error(not(source))] String),
}

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        match &self {
            Error::Field { source, .. } => source.as_response_error().status_code(),
            Error::ContentTypeIncompatible => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

#[cfg(test)]
mod tests {
    use err_report::Report;

    use super::*;

    #[test]
    fn test_multipart_error() {
        let resp = Error::BoundaryMissing.error_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let err = Error::Payload(PayloadError::Overflow);
        assert_eq!(
            Report::new(&err).to_string(),
            "Payload error: payload reached size limit",
        );

        let err = Error::Parse(ParseError::Incomplete);
        assert_eq!(
            Report::new(&err).to_string(),
            "Error during field parsing: message is incomplete"
        );

        let err = Error::ContentDispositionNameMissing;
        assert_eq!(
            Report::new(&err).to_string(),
            "Content-Disposition 'name' parameter was not found when parsing a \"form-data\" field"
        );

        let err = Error::Field {
            name: "avatar".to_owned(),
            source: actix_web::error::ErrorBadRequest("invalid file format"),
        };
        assert_eq!(
            Report::new(&err).to_string(),
            "An error occurred processing field \"avatar\": invalid file format"
        );
    }
}

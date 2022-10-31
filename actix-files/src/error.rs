use actix_web::{http::StatusCode, ResponseError};
use derive_more::Display;

/// Errors which can occur when serving static files.
#[derive(Debug, PartialEq, Eq, Display)]
pub enum FilesError {
    /// Path is not a directory
    #[allow(dead_code)]
    #[display(fmt = "Path is not a directory. Unable to serve static files")]
    IsNotDirectory,

    /// Cannot render directory
    #[display(fmt = "Unable to render directory without index file")]
    IsDirectory,
}

/// Return `NotFound` for `FilesError`
impl ResponseError for FilesError {
    fn status_code(&self) -> StatusCode {
        StatusCode::NOT_FOUND
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, PartialEq, Eq, Display)]
#[non_exhaustive]
pub enum UriSegmentError {
    /// The segment started with the wrapped invalid character.
    #[display(fmt = "The segment started with the wrapped invalid character")]
    BadStart(char),

    /// The segment contained the wrapped invalid character.
    #[display(fmt = "The segment contained the wrapped invalid character")]
    BadChar(char),

    /// The segment ended with the wrapped invalid character.
    #[display(fmt = "The segment ended with the wrapped invalid character")]
    BadEnd(char),

    /// The path is not a valid UTF-8 string after doing percent decoding.
    #[display(fmt = "The path is not a valid UTF-8 string after percent-decoding")]
    NotValidUtf8,
}

/// Return `BadRequest` for `UriSegmentError`
impl ResponseError for UriSegmentError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

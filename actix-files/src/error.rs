use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use thiserror::Error;

/// Errors which can occur when serving static files.
#[derive(Error, Debug, PartialEq)]
pub enum FilesError {
    /// Path is not a directory
    #[allow(dead_code)]
    #[error("Path is not a directory. Unable to serve static files")]
    IsNotDirectory,

    /// Cannot render directory
    #[error("Unable to render directory without index file")]
    IsDirectory,
}

/// Return `NotFound` for `FilesError`
impl ResponseError for FilesError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::NOT_FOUND)
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum UriSegmentError {
    /// The segment started with the wrapped invalid character.
    #[error("The segment started with the wrapped invalid character")]
    BadStart(char),
    /// The segment contained the wrapped invalid character.
    #[error("The segment contained the wrapped invalid character")]
    BadChar(char),
    /// The segment ended with the wrapped invalid character.
    #[error("The segment ended with the wrapped invalid character")]
    BadEnd(char),
}

/// Return `BadRequest` for `UriSegmentError`
impl ResponseError for UriSegmentError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

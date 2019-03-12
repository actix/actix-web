//! Error and Result module
use std::fmt;

pub use actix_http::error::*;
use derive_more::{Display, From};
use url::ParseError as UrlParseError;

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

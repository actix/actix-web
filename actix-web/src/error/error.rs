use std::{error::Error as StdError, fmt};

use actix_http::{body::BoxBody, Response};

use crate::{HttpResponse, ResponseError};

/// General purpose Actix Web error.
///
/// An Actix Web error is used to carry errors from `std::error` through Actix in a convenient way.
/// It can be created through converting errors with `into()`.
///
/// Whenever it is created from an external object a response error is created for it that can be
/// used to create an HTTP response from it this means that if you have access to an actix `Error`
/// you can always get a `ResponseError` reference from it.
pub struct Error {
    cause: Box<dyn ResponseError>,
    response_mappers: Vec<Box<dyn Fn(HttpResponse) -> HttpResponse>>,
}

impl Error {
    /// Returns the reference to the underlying `ResponseError`.
    pub fn as_response_error(&self) -> &dyn ResponseError {
        self.cause.as_ref()
    }

    /// Similar to `as_response_error` but downcasts.
    pub fn as_error<T: ResponseError + 'static>(&self) -> Option<&T> {
        <dyn ResponseError>::downcast_ref(self.cause.as_ref())
    }

    /// Shortcut for creating an `HttpResponse`.
    pub fn error_response(&self) -> HttpResponse {
        let mut res = self.cause.error_response();

        for mapper in &self.response_mappers {
            res = (mapper)(res);
        }

        res
    }

    /// Adds a function that maps the HTTP response generated for this error.
    ///
    /// Mappers are called in the order they are added each time
    /// [`error_response`](Self::error_response) is called. A mapper may receive a response already
    /// modified by other mappers, so it should avoid relying on a particular position in the chain.
    ///
    /// Prefer narrowly mutating the provided response. Preserve fields the mapper does not own,
    /// insert default headers only when absent, and merge list-valued headers without introducing
    /// duplicates. Mappers should also be deterministic and safe to call more than once.
    ///
    /// # Good
    ///
    /// This mapper preserves the error response and adds a default only when it is absent:
    ///
    /// ```
    /// use actix_web::{
    ///     error,
    ///     http::header::{self, HeaderValue},
    /// };
    ///
    /// let mut err = error::ErrorBadRequest("bad request");
    /// err.add_response_mapper(|mut res| {
    ///     if !res.headers().contains_key(header::CACHE_CONTROL) {
    ///         res.headers_mut()
    ///             .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    ///     }
    ///
    ///     res
    /// });
    ///
    /// assert_eq!(
    ///     err.error_response().headers().get(header::CACHE_CONTROL),
    ///     Some(&HeaderValue::from_static("no-store")),
    /// );
    /// ```
    ///
    /// # Bad
    ///
    /// Replacing the response discards the original status, body, headers, extensions, and any
    /// changes made by earlier mappers:
    ///
    /// ```
    /// use actix_web::{error, HttpResponse};
    ///
    /// let mut err = error::ErrorBadRequest("bad request");
    /// err.add_response_mapper(|_| HttpResponse::InternalServerError().finish());
    /// ```
    pub fn add_response_mapper<F>(&mut self, mapper: F)
    where
        F: Fn(HttpResponse) -> HttpResponse + 'static,
    {
        self.response_mappers.push(Box::new(mapper))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.cause)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

/// `Error` for any error that implements `ResponseError`
impl<T: ResponseError + 'static> From<T> for Error {
    fn from(err: T) -> Error {
        Error {
            cause: Box::new(err),
            response_mappers: Vec::new(),
        }
    }
}

impl From<Box<dyn ResponseError>> for Error {
    fn from(value: Box<dyn ResponseError>) -> Self {
        Error {
            cause: value,
            response_mappers: Vec::new(),
        }
    }
}

impl From<Error> for Response<BoxBody> {
    fn from(err: Error) -> Response<BoxBody> {
        err.error_response().into()
    }
}

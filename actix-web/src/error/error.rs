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

    pub fn add_mapper<F, B>(&mut self, mapper: F)
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
        Error { cause: value }
    }
}

impl From<Error> for Response<BoxBody> {
    fn from(err: Error) -> Response<BoxBody> {
        err.error_response().into()
    }
}

//! `ResponseError` trait and foreign impls.

use std::{
    convert::Infallible,
    error::Error as StdError,
    fmt,
    io::{self, Write as _},
};

use actix_http::Response;
use bytes::BytesMut;

use crate::{
    body::BoxBody,
    error::{downcast_dyn, downcast_get_type_id},
    helpers,
    http::{
        header::{self, TryIntoHeaderValue},
        StatusCode,
    },
    HttpResponse,
};

/// Errors that can generate responses.
// TODO: flesh out documentation
pub trait ResponseError: fmt::Debug + fmt::Display {
    /// Returns appropriate status code for error.
    ///
    /// A 500 Internal Server Error is used by default. If [error_response](Self::error_response) is
    /// also implemented and does not call `self.status_code()`, then this will not be used.
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    /// Creates full response for error.
    ///
    /// By default, the generated response uses a 500 Internal Server Error status code, a
    /// `Content-Type` of `text/plain`, and the body is set to `Self`'s `Display` impl.
    fn error_response(&self) -> HttpResponse<BoxBody> {
        let mut res = HttpResponse::new(self.status_code());

        let mut buf = BytesMut::new();
        let _ = write!(helpers::MutWriter(&mut buf), "{}", self);

        let mime = mime::TEXT_PLAIN_UTF_8.try_into_value().unwrap();
        res.headers_mut().insert(header::CONTENT_TYPE, mime);

        res.set_body(BoxBody::new(buf))
    }

    downcast_get_type_id!();
}

downcast_dyn!(ResponseError);

impl ResponseError for Box<dyn StdError + 'static> {}

impl ResponseError for Infallible {
    fn status_code(&self) -> StatusCode {
        match *self {}
    }
    fn error_response(&self) -> HttpResponse<BoxBody> {
        match *self {}
    }
}

#[cfg(feature = "openssl")]
impl ResponseError for actix_tls::accept::openssl::reexports::Error {}

impl ResponseError for serde::de::value::Error {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl ResponseError for serde_json::Error {}

impl ResponseError for serde_urlencoded::ser::Error {}

impl ResponseError for std::str::Utf8Error {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl ResponseError for std::io::Error {
    fn status_code(&self) -> StatusCode {
        match self.kind() {
            io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
            io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl ResponseError for actix_http::error::HttpError {}

impl ResponseError for actix_http::Error {
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn error_response(&self) -> HttpResponse<BoxBody> {
        HttpResponse::with_body(self.status_code(), self.to_string()).map_into_boxed_body()
    }
}

impl ResponseError for actix_http::header::InvalidHeaderValue {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl ResponseError for actix_http::error::ParseError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl ResponseError for actix_http::error::PayloadError {
    fn status_code(&self) -> StatusCode {
        match *self {
            actix_http::error::PayloadError::Overflow => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl ResponseError for actix_http::ws::ProtocolError {}

impl ResponseError for actix_http::error::ContentTypeError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

impl ResponseError for actix_http::ws::HandshakeError {
    fn error_response(&self) -> HttpResponse<BoxBody> {
        Response::from(self).map_into_boxed_body().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_casting() {
        use actix_http::error::{ContentTypeError, PayloadError};

        let err = PayloadError::Overflow;
        let resp_err: &dyn ResponseError = &err;

        let err = resp_err.downcast_ref::<PayloadError>().unwrap();
        assert_eq!(err.to_string(), "payload reached size limit");

        let not_err = resp_err.downcast_ref::<ContentTypeError>();
        assert!(not_err.is_none());
    }
}

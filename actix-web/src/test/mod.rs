//! Various helpers for Actix applications to use during testing.
//!
//! # Creating A Test Service
//! - [`init_service`]
//!
//! # Off-The-Shelf Test Services
//! - [`ok_service`]
//! - [`status_service`]
//!
//! # Calling Test Service
//! - [`TestRequest`]
//! - [`call_service`]
//! - [`try_call_service`]
//! - [`call_and_read_body`]
//! - [`call_and_read_body_json`]
//! - [`try_call_and_read_body_json`]
//!
//! # Reading Response Payloads
//! - [`read_body`]
//! - [`try_read_body`]
//! - [`read_body_json`]
//! - [`try_read_body_json`]

// TODO: more docs on generally how testing works with these parts

pub use actix_http::test::TestBuffer;

mod test_request;
mod test_services;
mod test_utils;

#[allow(deprecated)]
pub use self::test_services::{default_service, ok_service, simple_service, status_service};
#[cfg(test)]
pub(crate) use self::test_utils::try_init_service;
#[allow(deprecated)]
pub use self::test_utils::{read_response, read_response_json};
pub use self::{
    test_request::TestRequest,
    test_utils::{
        call_and_read_body, call_and_read_body_json, call_service, init_service, read_body,
        read_body_json, try_call_and_read_body_json, try_call_service, try_read_body,
        try_read_body_json,
    },
};

/// Reduces boilerplate code when testing expected response payloads.
///
/// Must be used inside an async test. Works for both `ServiceRequest` and `HttpRequest`.
///
/// # Examples
/// ```
/// use actix_web::{http::StatusCode, HttpResponse};
///
/// let res = HttpResponse::with_body(StatusCode::OK, "http response");
/// assert_body_eq!(res, b"http response");
/// ```
#[cfg(test)]
macro_rules! assert_body_eq {
    ($res:ident, $expected:expr) => {
        assert_eq!(
            ::actix_http::body::to_bytes($res.into_body())
                .await
                .expect("error reading test response body"),
            ::bytes::Bytes::from_static($expected),
        )
    };
}

#[cfg(test)]
pub(crate) use assert_body_eq;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{http::StatusCode, service::ServiceResponse, HttpResponse};

    #[actix_rt::test]
    async fn assert_body_works_for_service_and_regular_response() {
        let res = HttpResponse::with_body(StatusCode::OK, "http response");
        assert_body_eq!(res, b"http response");

        let req = TestRequest::default().to_http_request();
        let res = HttpResponse::with_body(StatusCode::OK, "service response");
        let res = ServiceResponse::new(req, res);
        assert_body_eq!(res, b"service response");
    }
}

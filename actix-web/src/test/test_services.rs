use actix_utils::future::ok;

use crate::{
    body::BoxBody,
    dev::{fn_service, Service, ServiceRequest, ServiceResponse},
    http::StatusCode,
    Error, HttpResponseBuilder,
};

/// Creates service that always responds with `200 OK` and no body.
pub fn ok_service(
) -> impl Service<ServiceRequest, Response = ServiceResponse<BoxBody>, Error = Error> {
    status_service(StatusCode::OK)
}

/// Creates service that always responds with given status code and no body.
pub fn status_service(
    status_code: StatusCode,
) -> impl Service<ServiceRequest, Response = ServiceResponse<BoxBody>, Error = Error> {
    fn_service(move |req: ServiceRequest| {
        ok(req.into_response(HttpResponseBuilder::new(status_code).finish()))
    })
}

#[doc(hidden)]
#[deprecated(since = "4.0.0", note = "Renamed to `status_service`.")]
pub fn simple_service(
    status_code: StatusCode,
) -> impl Service<ServiceRequest, Response = ServiceResponse<BoxBody>, Error = Error> {
    status_service(status_code)
}

#[doc(hidden)]
#[deprecated(since = "4.0.0", note = "Renamed to `status_service`.")]
pub fn default_service(
    status_code: StatusCode,
) -> impl Service<ServiceRequest, Response = ServiceResponse<BoxBody>, Error = Error> {
    status_service(status_code)
}

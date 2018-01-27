//! Basic http responses
#![allow(non_upper_case_globals)]
use http::{StatusCode, Error as HttpError};

use body::Body;
use handler::{Reply, Handler, RouteHandler, Responder};
use httprequest::HttpRequest;
use httpresponse::{HttpResponse, HttpResponseBuilder};

pub const HTTPOk: StaticResponse = StaticResponse(StatusCode::OK);
pub const HTTPCreated: StaticResponse = StaticResponse(StatusCode::CREATED);
pub const HTTPNoContent: StaticResponse = StaticResponse(StatusCode::NO_CONTENT);

pub const HTTPMultipleChoices: StaticResponse = StaticResponse(StatusCode::MULTIPLE_CHOICES);
pub const HTTPMovedPermanenty: StaticResponse = StaticResponse(StatusCode::MOVED_PERMANENTLY);
pub const HTTPFound: StaticResponse = StaticResponse(StatusCode::FOUND);
pub const HTTPSeeOther: StaticResponse = StaticResponse(StatusCode::SEE_OTHER);
pub const HTTPNotModified: StaticResponse = StaticResponse(StatusCode::NOT_MODIFIED);
pub const HTTPUseProxy: StaticResponse = StaticResponse(StatusCode::USE_PROXY);
pub const HTTPTemporaryRedirect: StaticResponse =
    StaticResponse(StatusCode::TEMPORARY_REDIRECT);
pub const HTTPPermanentRedirect: StaticResponse =
    StaticResponse(StatusCode::PERMANENT_REDIRECT);

pub const HTTPBadRequest: StaticResponse = StaticResponse(StatusCode::BAD_REQUEST);
pub const HTTPNotFound: StaticResponse = StaticResponse(StatusCode::NOT_FOUND);
pub const HTTPUnauthorized: StaticResponse = StaticResponse(StatusCode::UNAUTHORIZED);
pub const HTTPPaymentRequired: StaticResponse = StaticResponse(StatusCode::PAYMENT_REQUIRED);
pub const HTTPForbidden: StaticResponse = StaticResponse(StatusCode::FORBIDDEN);

pub const HTTPMethodNotAllowed: StaticResponse =
    StaticResponse(StatusCode::METHOD_NOT_ALLOWED);
pub const HTTPNotAcceptable: StaticResponse = StaticResponse(StatusCode::NOT_ACCEPTABLE);
pub const HTTPProxyAuthenticationRequired: StaticResponse =
    StaticResponse(StatusCode::PROXY_AUTHENTICATION_REQUIRED);
pub const HTTPRequestTimeout: StaticResponse = StaticResponse(StatusCode::REQUEST_TIMEOUT);
pub const HTTPConflict: StaticResponse = StaticResponse(StatusCode::CONFLICT);
pub const HTTPGone: StaticResponse = StaticResponse(StatusCode::GONE);
pub const HTTPLengthRequired: StaticResponse = StaticResponse(StatusCode::LENGTH_REQUIRED);
pub const HTTPPreconditionFailed: StaticResponse =
    StaticResponse(StatusCode::PRECONDITION_FAILED);
pub const HTTPPayloadTooLarge: StaticResponse = StaticResponse(StatusCode::PAYLOAD_TOO_LARGE);
pub const HTTPUriTooLong: StaticResponse = StaticResponse(StatusCode::URI_TOO_LONG);
pub const HTTPExpectationFailed: StaticResponse =
    StaticResponse(StatusCode::EXPECTATION_FAILED);

pub const HTTPInternalServerError: StaticResponse =
    StaticResponse(StatusCode::INTERNAL_SERVER_ERROR);


#[derive(Copy, Clone, Debug)]
pub struct StaticResponse(StatusCode);

impl StaticResponse {
    pub fn build(&self) -> HttpResponseBuilder {
        HttpResponse::build(self.0)
    }
    pub fn with_reason(self, reason: &'static str) -> HttpResponse {
        let mut resp = HttpResponse::new(self.0, Body::Empty);
        resp.set_reason(reason);
        resp
    }
    pub fn with_body<B: Into<Body>>(self, body: B) -> HttpResponse {
        HttpResponse::new(self.0, body.into())
    }
}

impl<S> Handler<S> for StaticResponse {
    type Result = HttpResponse;

    fn handle(&mut self, _: HttpRequest<S>) -> HttpResponse {
        HttpResponse::new(self.0, Body::Empty)
    }
}

impl<S> RouteHandler<S> for StaticResponse {
    fn handle(&mut self, _: HttpRequest<S>) -> Reply {
        Reply::response(HttpResponse::new(self.0, Body::Empty))
    }
}

impl Responder for StaticResponse {
    type Item = HttpResponse;
    type Error = HttpError;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.build().body(Body::Empty)
    }
}

impl From<StaticResponse> for HttpResponse {
    fn from(st: StaticResponse) -> Self {
        HttpResponse::new(st.0, Body::Empty)
    }
}

impl From<StaticResponse> for Reply {
    fn from(st: StaticResponse) -> Self {
        HttpResponse::new(st.0, Body::Empty).into()
    }
}

macro_rules! STATIC_RESP {
    ($name:ident, $status:expr) => {
        #[allow(non_snake_case)]
        pub fn $name() -> HttpResponseBuilder {
            HttpResponse::build($status)
        }
    }
}

impl HttpResponse {
    STATIC_RESP!(Ok, StatusCode::OK);
    STATIC_RESP!(Created, StatusCode::CREATED);
    STATIC_RESP!(NoContent, StatusCode::NO_CONTENT);

    STATIC_RESP!(MultipleChoices, StatusCode::MULTIPLE_CHOICES);
    STATIC_RESP!(MovedPermanenty, StatusCode::MOVED_PERMANENTLY);
    STATIC_RESP!(Found, StatusCode::FOUND);
    STATIC_RESP!(SeeOther, StatusCode::SEE_OTHER);
    STATIC_RESP!(NotModified, StatusCode::NOT_MODIFIED);
    STATIC_RESP!(UseProxy, StatusCode::USE_PROXY);
    STATIC_RESP!(TemporaryRedirect, StatusCode::TEMPORARY_REDIRECT);
    STATIC_RESP!(PermanentRedirect, StatusCode::PERMANENT_REDIRECT);

    STATIC_RESP!(BadRequest, StatusCode::BAD_REQUEST);
    STATIC_RESP!(NotFound, StatusCode::NOT_FOUND);
    STATIC_RESP!(Unauthorized, StatusCode::UNAUTHORIZED);
    STATIC_RESP!(PaymentRequired, StatusCode::PAYMENT_REQUIRED);
    STATIC_RESP!(Forbidden, StatusCode::FORBIDDEN);

    STATIC_RESP!(MethodNotAllowed, StatusCode::METHOD_NOT_ALLOWED);
    STATIC_RESP!(NotAcceptable, StatusCode::NOT_ACCEPTABLE);
    STATIC_RESP!(ProxyAuthenticationRequired, StatusCode::PROXY_AUTHENTICATION_REQUIRED);
    STATIC_RESP!(RequestTimeout, StatusCode::REQUEST_TIMEOUT);
    STATIC_RESP!(Conflict, StatusCode::CONFLICT);
    STATIC_RESP!(Gone, StatusCode::GONE);
    STATIC_RESP!(LengthRequired, StatusCode::LENGTH_REQUIRED);
    STATIC_RESP!(PreconditionFailed, StatusCode::PRECONDITION_FAILED);
    STATIC_RESP!(PayloadTooLarge, StatusCode::PAYLOAD_TOO_LARGE);
    STATIC_RESP!(UriTooLong, StatusCode::URI_TOO_LONG);
    STATIC_RESP!(ExpectationFailed, StatusCode::EXPECTATION_FAILED);

    STATIC_RESP!(InternalServerError, StatusCode::INTERNAL_SERVER_ERROR);
}

#[cfg(test)]
mod tests {
    use http::StatusCode;
    use super::{HTTPOk, HTTPBadRequest, Body, HttpResponse};

    #[test]
    fn test_build() {
        let resp = HTTPOk.build().body(Body::Empty).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_response() {
        let resp: HttpResponse = HTTPOk.into();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_from() {
        let resp: HttpResponse = HTTPOk.into();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_with_reason() {
        let resp: HttpResponse = HTTPOk.into();
        assert_eq!(resp.reason(), "OK");

        let resp = HTTPBadRequest.with_reason("test");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(resp.reason(), "test");
    }
}

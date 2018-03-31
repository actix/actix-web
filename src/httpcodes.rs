//! Basic http responses
#![allow(non_upper_case_globals)]
use http::StatusCode;

use body::Body;
use error::Error;
use handler::{Reply, Handler, RouteHandler, Responder};
use httprequest::HttpRequest;
use httpresponse::{HttpResponse, HttpResponseBuilder};

pub const HttpOk: StaticResponse = StaticResponse(StatusCode::OK);
pub const HttpCreated: StaticResponse = StaticResponse(StatusCode::CREATED);
pub const HttpAccepted: StaticResponse = StaticResponse(StatusCode::ACCEPTED);
pub const HttpNonAuthoritativeInformation: StaticResponse =
    StaticResponse(StatusCode::NON_AUTHORITATIVE_INFORMATION);
pub const HttpNoContent: StaticResponse = StaticResponse(StatusCode::NO_CONTENT);
pub const HttpResetContent: StaticResponse = StaticResponse(StatusCode::RESET_CONTENT);
pub const HttpPartialContent: StaticResponse = StaticResponse(StatusCode::PARTIAL_CONTENT);
pub const HttpMultiStatus: StaticResponse = StaticResponse(StatusCode::MULTI_STATUS);
pub const HttpAlreadyReported: StaticResponse = StaticResponse(StatusCode::ALREADY_REPORTED);

pub const HttpMultipleChoices: StaticResponse = StaticResponse(StatusCode::MULTIPLE_CHOICES);
pub const HttpMovedPermanently: StaticResponse = StaticResponse(StatusCode::MOVED_PERMANENTLY);
pub const HttpFound: StaticResponse = StaticResponse(StatusCode::FOUND);
pub const HttpSeeOther: StaticResponse = StaticResponse(StatusCode::SEE_OTHER);
pub const HttpNotModified: StaticResponse = StaticResponse(StatusCode::NOT_MODIFIED);
pub const HttpUseProxy: StaticResponse = StaticResponse(StatusCode::USE_PROXY);
pub const HttpTemporaryRedirect: StaticResponse =
    StaticResponse(StatusCode::TEMPORARY_REDIRECT);
pub const HttpPermanentRedirect: StaticResponse =
    StaticResponse(StatusCode::PERMANENT_REDIRECT);

pub const HttpBadRequest: StaticResponse = StaticResponse(StatusCode::BAD_REQUEST);
pub const HttpUnauthorized: StaticResponse = StaticResponse(StatusCode::UNAUTHORIZED);
pub const HttpPaymentRequired: StaticResponse = StaticResponse(StatusCode::PAYMENT_REQUIRED);
pub const HttpForbidden: StaticResponse = StaticResponse(StatusCode::FORBIDDEN);
pub const HttpNotFound: StaticResponse = StaticResponse(StatusCode::NOT_FOUND);
pub const HttpMethodNotAllowed: StaticResponse =
    StaticResponse(StatusCode::METHOD_NOT_ALLOWED);
pub const HttpNotAcceptable: StaticResponse = StaticResponse(StatusCode::NOT_ACCEPTABLE);
pub const HttpProxyAuthenticationRequired: StaticResponse =
    StaticResponse(StatusCode::PROXY_AUTHENTICATION_REQUIRED);
pub const HttpRequestTimeout: StaticResponse = StaticResponse(StatusCode::REQUEST_TIMEOUT);
pub const HttpConflict: StaticResponse = StaticResponse(StatusCode::CONFLICT);
pub const HttpGone: StaticResponse = StaticResponse(StatusCode::GONE);
pub const HttpLengthRequired: StaticResponse = StaticResponse(StatusCode::LENGTH_REQUIRED);
pub const HttpPreconditionFailed: StaticResponse =
    StaticResponse(StatusCode::PRECONDITION_FAILED);
pub const HttpPayloadTooLarge: StaticResponse = StaticResponse(StatusCode::PAYLOAD_TOO_LARGE);
pub const HttpUriTooLong: StaticResponse = StaticResponse(StatusCode::URI_TOO_LONG);
pub const HttpUnsupportedMediaType: StaticResponse =
    StaticResponse(StatusCode::UNSUPPORTED_MEDIA_TYPE);
pub const HttpRangeNotSatisfiable: StaticResponse =
    StaticResponse(StatusCode::RANGE_NOT_SATISFIABLE);
pub const HttpExpectationFailed: StaticResponse =
    StaticResponse(StatusCode::EXPECTATION_FAILED);

pub const HttpInternalServerError: StaticResponse =
    StaticResponse(StatusCode::INTERNAL_SERVER_ERROR);
pub const HttpNotImplemented: StaticResponse = StaticResponse(StatusCode::NOT_IMPLEMENTED);
pub const HttpBadGateway: StaticResponse = StaticResponse(StatusCode::BAD_GATEWAY);
pub const HttpServiceUnavailable: StaticResponse =
    StaticResponse(StatusCode::SERVICE_UNAVAILABLE);
pub const HttpGatewayTimeout: StaticResponse =
    StaticResponse(StatusCode::GATEWAY_TIMEOUT);
pub const HttpVersionNotSupported: StaticResponse =
    StaticResponse(StatusCode::HTTP_VERSION_NOT_SUPPORTED);
pub const HttpVariantAlsoNegotiates: StaticResponse =
    StaticResponse(StatusCode::VARIANT_ALSO_NEGOTIATES);
pub const HttpInsufficientStorage: StaticResponse =
    StaticResponse(StatusCode::INSUFFICIENT_STORAGE);
pub const HttpLoopDetected: StaticResponse = StaticResponse(StatusCode::LOOP_DETECTED);


#[derive(Copy, Clone, Debug)]
pub struct StaticResponse(StatusCode);

impl StaticResponse {
    pub fn build(&self) -> HttpResponseBuilder {
        HttpResponse::build(self.0)
    }
    pub fn build_from<S>(&self, req: &HttpRequest<S>) -> HttpResponseBuilder {
        req.build_response(self.0)
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
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, Error> {
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
    use super::{HttpOk, HttpBadRequest, Body, HttpResponse};

    #[test]
    fn test_build() {
        let resp = HttpOk.build().body(Body::Empty).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_response() {
        let resp: HttpResponse = HttpOk.into();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_from() {
        let resp: HttpResponse = HttpOk.into();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_with_reason() {
        let resp: HttpResponse = HttpOk.into();
        assert_eq!(resp.reason(), "OK");

        let resp = HttpBadRequest.with_reason("test");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(resp.reason(), "test");
    }
}

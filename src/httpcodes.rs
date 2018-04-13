//! Basic http responses
#![allow(non_upper_case_globals, deprecated)]
use http::StatusCode;

use body::Body;
use error::Error;
use handler::{Reply, Handler, RouteHandler, Responder};
use httprequest::HttpRequest;
use httpresponse::{HttpResponse, HttpResponseBuilder};

#[deprecated(since="0.5.0", note="please use `HttpResponse::Ok()` instead")]
pub const HttpOk: StaticResponse = StaticResponse(StatusCode::OK);
#[deprecated(since="0.5.0", note="please use `HttpResponse::Created()` instead")]
pub const HttpCreated: StaticResponse = StaticResponse(StatusCode::CREATED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::Accepted()` instead")]
pub const HttpAccepted: StaticResponse = StaticResponse(StatusCode::ACCEPTED);
#[deprecated(since="0.5.0",
             note="please use `HttpResponse::pNonAuthoritativeInformation()` instead")]
pub const HttpNonAuthoritativeInformation: StaticResponse =
    StaticResponse(StatusCode::NON_AUTHORITATIVE_INFORMATION);
#[deprecated(since="0.5.0", note="please use `HttpResponse::NoContent()` instead")]
pub const HttpNoContent: StaticResponse = StaticResponse(StatusCode::NO_CONTENT);
#[deprecated(since="0.5.0", note="please use `HttpResponse::ResetContent()` instead")]
pub const HttpResetContent: StaticResponse = StaticResponse(StatusCode::RESET_CONTENT);
#[deprecated(since="0.5.0", note="please use `HttpResponse::PartialContent()` instead")]
pub const HttpPartialContent: StaticResponse = StaticResponse(StatusCode::PARTIAL_CONTENT);
#[deprecated(since="0.5.0", note="please use `HttpResponse::MultiStatus()` instead")]
pub const HttpMultiStatus: StaticResponse = StaticResponse(StatusCode::MULTI_STATUS);
#[deprecated(since="0.5.0", note="please use `HttpResponse::AlreadyReported()` instead")]
pub const HttpAlreadyReported: StaticResponse = StaticResponse(StatusCode::ALREADY_REPORTED);

#[deprecated(since="0.5.0", note="please use `HttpResponse::MultipleChoices()` instead")]
pub const HttpMultipleChoices: StaticResponse = StaticResponse(StatusCode::MULTIPLE_CHOICES);
#[deprecated(since="0.5.0", note="please use `HttpResponse::MovedPermanently()` instead")]
pub const HttpMovedPermanently: StaticResponse = StaticResponse(StatusCode::MOVED_PERMANENTLY);
#[deprecated(since="0.5.0", note="please use `HttpResponse::Found()` instead")]
pub const HttpFound: StaticResponse = StaticResponse(StatusCode::FOUND);
#[deprecated(since="0.5.0", note="please use `HttpResponse::SeeOther()` instead")]
pub const HttpSeeOther: StaticResponse = StaticResponse(StatusCode::SEE_OTHER);
#[deprecated(since="0.5.0", note="please use `HttpResponse::NotModified()` instead")]
pub const HttpNotModified: StaticResponse = StaticResponse(StatusCode::NOT_MODIFIED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::UseProxy()` instead")]
pub const HttpUseProxy: StaticResponse = StaticResponse(StatusCode::USE_PROXY);
#[deprecated(since="0.5.0", note="please use `HttpResponse::TemporaryRedirect()` instead")]
pub const HttpTemporaryRedirect: StaticResponse =
    StaticResponse(StatusCode::TEMPORARY_REDIRECT);
#[deprecated(since="0.5.0", note="please use `HttpResponse::PermanentRedirect()` instead")]
pub const HttpPermanentRedirect: StaticResponse =
    StaticResponse(StatusCode::PERMANENT_REDIRECT);

#[deprecated(since="0.5.0", note="please use `HttpResponse::BadRequest()` instead")]
pub const HttpBadRequest: StaticResponse = StaticResponse(StatusCode::BAD_REQUEST);
#[deprecated(since="0.5.0", note="please use `HttpResponse::Unauthorized()` instead")]
pub const HttpUnauthorized: StaticResponse = StaticResponse(StatusCode::UNAUTHORIZED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::PaymentRequired()` instead")]
pub const HttpPaymentRequired: StaticResponse = StaticResponse(StatusCode::PAYMENT_REQUIRED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::Forbidden()` instead")]
pub const HttpForbidden: StaticResponse = StaticResponse(StatusCode::FORBIDDEN);
#[deprecated(since="0.5.0", note="please use `HttpResponse::NotFound()` instead")]
pub const HttpNotFound: StaticResponse = StaticResponse(StatusCode::NOT_FOUND);
#[deprecated(since="0.5.0", note="please use `HttpResponse::MethodNotAllowed()` instead")]
pub const HttpMethodNotAllowed: StaticResponse =
    StaticResponse(StatusCode::METHOD_NOT_ALLOWED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::NotAcceptable()` instead")]
pub const HttpNotAcceptable: StaticResponse = StaticResponse(StatusCode::NOT_ACCEPTABLE);
#[deprecated(since="0.5.0",
             note="please use `HttpResponse::ProxyAuthenticationRequired()` instead")]
pub const HttpProxyAuthenticationRequired: StaticResponse =
    StaticResponse(StatusCode::PROXY_AUTHENTICATION_REQUIRED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::RequestTimeout()` instead")]
pub const HttpRequestTimeout: StaticResponse = StaticResponse(StatusCode::REQUEST_TIMEOUT);
#[deprecated(since="0.5.0", note="please use `HttpResponse::Conflict()` instead")]
pub const HttpConflict: StaticResponse = StaticResponse(StatusCode::CONFLICT);
#[deprecated(since="0.5.0", note="please use `HttpResponse::Gone()` instead")]
pub const HttpGone: StaticResponse = StaticResponse(StatusCode::GONE);
#[deprecated(since="0.5.0", note="please use `HttpResponse::LengthRequired()` instead")]
pub const HttpLengthRequired: StaticResponse = StaticResponse(StatusCode::LENGTH_REQUIRED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::PreconditionFailed()` instead")]
pub const HttpPreconditionFailed: StaticResponse =
    StaticResponse(StatusCode::PRECONDITION_FAILED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::PayloadTooLarge()` instead")]
pub const HttpPayloadTooLarge: StaticResponse = StaticResponse(StatusCode::PAYLOAD_TOO_LARGE);
#[deprecated(since="0.5.0", note="please use `HttpResponse::UriTooLong()` instead")]
pub const HttpUriTooLong: StaticResponse = StaticResponse(StatusCode::URI_TOO_LONG);
#[deprecated(since="0.5.0",
             note="please use `HttpResponse::UnsupportedMediaType()` instead")]
pub const HttpUnsupportedMediaType: StaticResponse =
    StaticResponse(StatusCode::UNSUPPORTED_MEDIA_TYPE);
#[deprecated(since="0.5.0",
             note="please use `HttpResponse::RangeNotSatisfiable()` instead")]
pub const HttpRangeNotSatisfiable: StaticResponse =
    StaticResponse(StatusCode::RANGE_NOT_SATISFIABLE);
#[deprecated(since="0.5.0", note="please use `HttpResponse::ExpectationFailed()` instead")]
pub const HttpExpectationFailed: StaticResponse =
    StaticResponse(StatusCode::EXPECTATION_FAILED);

#[deprecated(since="0.5.0",
             note="please use `HttpResponse::InternalServerError()` instead")]
pub const HttpInternalServerError: StaticResponse =
    StaticResponse(StatusCode::INTERNAL_SERVER_ERROR);
#[deprecated(since="0.5.0", note="please use `HttpResponse::NotImplemented()` instead")]
pub const HttpNotImplemented: StaticResponse = StaticResponse(StatusCode::NOT_IMPLEMENTED);
#[deprecated(since="0.5.0", note="please use `HttpResponse::BadGateway()` instead")]
pub const HttpBadGateway: StaticResponse = StaticResponse(StatusCode::BAD_GATEWAY);
#[deprecated(since="0.5.0", note="please use `HttpResponse::ServiceUnavailable()` instead")]
pub const HttpServiceUnavailable: StaticResponse =
    StaticResponse(StatusCode::SERVICE_UNAVAILABLE);
#[deprecated(since="0.5.0", note="please use `HttpResponse::GatewayTimeout()` instead")]
pub const HttpGatewayTimeout: StaticResponse =
    StaticResponse(StatusCode::GATEWAY_TIMEOUT);
#[deprecated(since="0.5.0",
             note="please use `HttpResponse::VersionNotSupported()` instead")]
pub const HttpVersionNotSupported: StaticResponse =
    StaticResponse(StatusCode::HTTP_VERSION_NOT_SUPPORTED);
#[deprecated(since="0.5.0",
             note="please use `HttpResponse::VariantAlsoNegotiates()` instead")]
pub const HttpVariantAlsoNegotiates: StaticResponse =
    StaticResponse(StatusCode::VARIANT_ALSO_NEGOTIATES);
#[deprecated(since="0.5.0",
             note="please use `HttpResponse::InsufficientStorage()` instead")]
pub const HttpInsufficientStorage: StaticResponse =
    StaticResponse(StatusCode::INSUFFICIENT_STORAGE);
#[deprecated(since="0.5.0", note="please use `HttpResponse::LoopDetected()` instead")]
pub const HttpLoopDetected: StaticResponse = StaticResponse(StatusCode::LOOP_DETECTED);


#[deprecated(since="0.5.0", note="please use `HttpResponse` instead")]
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
        let mut resp = HttpResponse::new(self.0);
        resp.set_reason(reason);
        resp
    }
    pub fn with_body<B: Into<Body>>(self, body: B) -> HttpResponse {
        HttpResponse::with_body(self.0, body.into())
    }
}

impl<S> Handler<S> for StaticResponse {
    type Result = HttpResponse;

    fn handle(&mut self, _: HttpRequest<S>) -> HttpResponse {
        HttpResponse::new(self.0)
    }
}

impl<S> RouteHandler<S> for StaticResponse {
    fn handle(&mut self, _: HttpRequest<S>) -> Reply {
        Reply::response(HttpResponse::new(self.0))
    }
}

impl Responder for StaticResponse {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(self.build().finish())
    }
}

impl From<StaticResponse> for HttpResponse {
    fn from(st: StaticResponse) -> Self {
        HttpResponse::new(st.0)
    }
}

impl From<StaticResponse> for Reply {
    fn from(st: StaticResponse) -> Self {
        HttpResponse::new(st.0).into()
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
    STATIC_RESP!(Accepted, StatusCode::ACCEPTED);
    STATIC_RESP!(NonAuthoritativeInformation, StatusCode::NON_AUTHORITATIVE_INFORMATION);

    STATIC_RESP!(NoContent, StatusCode::NO_CONTENT);
    STATIC_RESP!(ResetContent, StatusCode::RESET_CONTENT);
    STATIC_RESP!(PartialContent, StatusCode::PARTIAL_CONTENT);
    STATIC_RESP!(MultiStatus, StatusCode::MULTI_STATUS);
    STATIC_RESP!(AlreadyReported, StatusCode::ALREADY_REPORTED);

    STATIC_RESP!(MultipleChoices, StatusCode::MULTIPLE_CHOICES);
    STATIC_RESP!(MovedPermanenty, StatusCode::MOVED_PERMANENTLY);
    STATIC_RESP!(MovedPermanently, StatusCode::MOVED_PERMANENTLY);
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
    STATIC_RESP!(UnsupportedMediaType, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    STATIC_RESP!(RangeNotSatisfiable, StatusCode::RANGE_NOT_SATISFIABLE);
    STATIC_RESP!(ExpectationFailed, StatusCode::EXPECTATION_FAILED);

    STATIC_RESP!(InternalServerError, StatusCode::INTERNAL_SERVER_ERROR);
    STATIC_RESP!(NotImplemented, StatusCode::NOT_IMPLEMENTED);
    STATIC_RESP!(BadGateway, StatusCode::BAD_GATEWAY);
    STATIC_RESP!(ServiceUnavailable, StatusCode::SERVICE_UNAVAILABLE);
    STATIC_RESP!(GatewayTimeout, StatusCode::GATEWAY_TIMEOUT);
    STATIC_RESP!(VersionNotSupported, StatusCode::HTTP_VERSION_NOT_SUPPORTED);
    STATIC_RESP!(VariantAlsoNegotiates, StatusCode::VARIANT_ALSO_NEGOTIATES);
    STATIC_RESP!(InsufficientStorage, StatusCode::INSUFFICIENT_STORAGE);
    STATIC_RESP!(LoopDetected, StatusCode::LOOP_DETECTED);
}

#[cfg(test)]
mod tests {
    use http::StatusCode;
    use super::{HttpOk, HttpBadRequest, Body, HttpResponse};

    #[test]
    fn test_build() {
        let resp = HttpOk.build().body(Body::Empty);
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

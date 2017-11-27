//! Basic http responses
#![allow(non_upper_case_globals)]
use std::rc::Rc;
use http::StatusCode;

use body::Body;
use task::Task;
use route::RouteHandler;
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


pub struct StaticResponse(StatusCode);

impl StaticResponse {
    pub fn builder(&self) -> HttpResponseBuilder {
        HttpResponse::builder(self.0)
    }
    pub fn response(&self) -> HttpResponse {
        HttpResponse::new(self.0, Body::Empty)
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

impl<S> RouteHandler<S> for StaticResponse {
    fn handle(&self, _: HttpRequest, _: Rc<S>) -> Task {
        Task::reply(HttpResponse::new(self.0, Body::Empty))
    }
}

impl From<StaticResponse> for HttpResponse {
    fn from(st: StaticResponse) -> Self {
        st.response()
    }
}


#[cfg(test)]
mod tests {
    use http::StatusCode;
    use super::{HTTPOk, HTTPBadRequest, Body, HttpResponse};

    #[test]
    fn test_builder() {
        let resp = HTTPOk.builder().body(Body::Empty).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_response() {
        let resp = HTTPOk.response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_from() {
        let resp: HttpResponse = HTTPOk.into();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_with_reason() {
        let resp = HTTPOk.response();
        assert_eq!(resp.reason(), "");

        let resp = HTTPBadRequest.with_reason("test");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(resp.reason(), "test");
    }
}

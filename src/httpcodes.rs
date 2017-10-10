//! Basic http responses
#![allow(non_upper_case_globals)]
use std::rc::Rc;
use http::StatusCode;

use task::Task;
use route::RouteHandler;
use payload::Payload;
use httpmessage::{Body, Builder, HttpRequest, HttpResponse};

pub const HTTPOk: StaticResponse = StaticResponse(StatusCode::OK);
pub const HTTPCreated: StaticResponse = StaticResponse(StatusCode::CREATED);
pub const HTTPNoContent: StaticResponse = StaticResponse(StatusCode::NO_CONTENT);
pub const HTTPBadRequest: StaticResponse = StaticResponse(StatusCode::BAD_REQUEST);
pub const HTTPNotFound: StaticResponse = StaticResponse(StatusCode::NOT_FOUND);
pub const HTTPMethodNotAllowed: StaticResponse = StaticResponse(StatusCode::METHOD_NOT_ALLOWED);
pub const HTTPInternalServerError: StaticResponse =
    StaticResponse(StatusCode::INTERNAL_SERVER_ERROR);


pub struct StaticResponse(StatusCode);

impl StaticResponse {
    pub fn builder(&self) -> Builder {
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
}

impl<S> RouteHandler<S> for StaticResponse {
    fn handle(&self, req: HttpRequest, _: Payload, _: Rc<S>) -> Task {
        Task::reply(req, HttpResponse::new(self.0, Body::Empty))
    }
}

impl From<StaticResponse> for HttpResponse {
    fn from(st: StaticResponse) -> Self {
        st.response()
    }
}

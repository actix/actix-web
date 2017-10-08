//! Basic http responses
#![allow(non_upper_case_globals)]
use std::rc::Rc;
use http::StatusCode;

use task::Task;
use route::{Payload, RouteHandler};
use httpmessage::{Body, HttpRequest, HttpResponse, IntoHttpResponse};

pub const HTTPOk: StaticResponse = StaticResponse(StatusCode::OK);
pub const HTTPCreated: StaticResponse = StaticResponse(StatusCode::CREATED);
pub const HTTPNoContent: StaticResponse = StaticResponse(StatusCode::NO_CONTENT);
pub const HTTPBadRequest: StaticResponse = StaticResponse(StatusCode::BAD_REQUEST);
pub const HTTPNotFound: StaticResponse = StaticResponse(StatusCode::NOT_FOUND);
pub const HTTPMethodNotAllowed: StaticResponse = StaticResponse(StatusCode::METHOD_NOT_ALLOWED);


pub struct StaticResponse(StatusCode);

impl StaticResponse {
    pub fn with_reason(self, req: HttpRequest, reason: &'static str) -> HttpResponse {
        HttpResponse::new(req, self.0, Body::Empty)
            .set_reason(reason)
    }
}

impl<S> RouteHandler<S> for StaticResponse {
    fn handle(&self, req: HttpRequest, _: Option<Payload>, _: Rc<S>) -> Task
    {
        Task::reply(HttpResponse::new(req, self.0, Body::Empty))
    }
}

impl IntoHttpResponse for StaticResponse {
    fn response(self, req: HttpRequest) -> HttpResponse {
        HttpResponse::new(req, self.0, Body::Empty)
    }
}

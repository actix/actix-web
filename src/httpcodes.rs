//! Basic http responses
#![allow(non_upper_case_globals)]
use std::rc::Rc;
use http::StatusCode;

use task::Task;
use route::{Payload, RouteHandler};
use httpmessage::{Body, HttpRequest, HttpMessage, IntoHttpMessage};

pub struct StaticResponse(StatusCode);

pub const HTTPOk: StaticResponse = StaticResponse(StatusCode::OK);
pub const HTTPCreated: StaticResponse = StaticResponse(StatusCode::CREATED);
pub const HTTPNoContent: StaticResponse = StaticResponse(StatusCode::NO_CONTENT);
pub const HTTPBadRequest: StaticResponse = StaticResponse(StatusCode::BAD_REQUEST);
pub const HTTPNotFound: StaticResponse = StaticResponse(StatusCode::NOT_FOUND);
pub const HTTPMethodNotAllowed: StaticResponse = StaticResponse(StatusCode::METHOD_NOT_ALLOWED);


impl<S> RouteHandler<S> for StaticResponse {
    fn handle(&self, req: HttpRequest, _: Option<Payload>, _: Rc<S>) -> Task
    {
        Task::reply(HttpMessage::new(req, self.0, Body::Empty), None)
    }
}

impl IntoHttpMessage for StaticResponse {
    fn into_response(self, req: HttpRequest) -> HttpMessage {
        HttpMessage::new(req, self.0, Body::Empty)
    }
}

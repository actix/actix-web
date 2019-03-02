#![allow(clippy::type_complexity, dead_code, unused_variables)]

mod app;
mod extractor;
pub mod handler;
// mod helpers;
// mod info;
pub mod blocking;
pub mod filter;
pub mod middleware;
mod request;
mod resource;
mod responder;
mod route;
mod service;
mod state;

// re-export for convenience
pub use actix_http::Response as HttpResponse;
pub use actix_http::{http, Error, HttpMessage, ResponseError};

pub use crate::app::App;
pub use crate::extractor::{Form, Json, Path, Query};
pub use crate::handler::FromRequest;
pub use crate::request::HttpRequest;
pub use crate::resource::Resource;
pub use crate::responder::{Either, Responder};
pub use crate::service::{ServiceRequest, ServiceResponse};
pub use crate::state::State;

pub mod dev {
    pub use crate::app::AppService;
    pub use crate::handler::{AsyncFactory, Extract, Factory, Handle};
    pub use crate::route::{Route, RouteBuilder};
    // pub use crate::info::ConnectionInfo;
}

//! The `actix-http` prelude for library developers
//!
//! The purpose of this module is to alleviate imports of many common actix traits
//! by adding a glob import to the top of actix heavy modules:
//!
//! ```
//! # #![allow(unused_imports)]
//! use actix_http::dev::*;
//! ```
pub use ws;
pub use httpcodes;
pub use application::Application;
pub use httpmessage::{Body, Builder, HttpRequest, HttpResponse};
pub use payload::{Payload, PayloadItem};
pub use router::RoutingMap;
pub use resource::{Reply, Resource};
pub use route::{Route, RouteFactory, RouteHandler};
pub use server::HttpServer;
pub use context::HttpContext;
pub use task::Task;
pub use route_recognizer::Params;

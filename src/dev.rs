//! The `actix-web` prelude for library developers
//!
//! The purpose of this module is to alleviate imports of many common actix traits
//! by adding a glob import to the top of actix heavy modules:
//!
//! ```
//! # #![allow(unused_imports)]
//! use actix_web::dev::*;
//! ```
pub use ws;
pub use httpcodes;
pub use error::ParseError;
pub use application::Application;
pub use httprequest::HttpRequest;
pub use httpmessage::{Body, Builder, HttpResponse};
pub use payload::{Payload, PayloadItem, PayloadError};
pub use router::RoutingMap;
pub use resource::{Reply, Resource};
pub use route::{Route, RouteFactory, RouteHandler};
pub use server::HttpServer;
pub use context::HttpContext;
pub use staticfiles::StaticFiles;

// re-exports
pub use cookie::{Cookie, CookieBuilder};
pub use cookie::{ParseError as CookieParseError};
pub use route_recognizer::Params;
pub use http_range::{HttpRange, HttpRangeParseError};

// dev specific
pub use task::Task;

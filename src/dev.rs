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
pub use application::{Application, ApplicationBuilder};
pub use httprequest::HttpRequest;
pub use httpresponse::{Body, HttpResponse, HttpResponseBuilder};
pub use payload::{Payload, PayloadItem, PayloadError};
pub use resource::{Reply, Resource};
pub use route::{Route, RouteFactory, RouteHandler};
pub use recognizer::Params;
pub use server::HttpServer;
pub use context::HttpContext;
pub use staticfiles::StaticFiles;

// re-exports
pub use http::{Method, StatusCode};
pub use cookie::{Cookie, CookieBuilder};
pub use cookie::{ParseError as CookieParseError};
pub use http_range::{HttpRange, HttpRangeParseError};

// dev specific
pub use task::Task;

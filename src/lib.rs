//! Http framework for [Actix](https://github.com/fafhrd91/actix)

#![cfg_attr(feature="nightly", feature(
    try_trait, // std::ops::Try #42327
))]

#[macro_use]
extern crate log;
extern crate time;
extern crate bytes;
extern crate sha1;
extern crate regex;
#[macro_use]
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;

extern crate cookie;
extern crate http;
extern crate httparse;
extern crate http_range;
extern crate mime_guess;
extern crate url;
extern crate actix;

mod application;
mod context;
mod error;
mod date;
mod decode;
mod httprequest;
mod httpresponse;
mod payload;
mod resource;
mod recognizer;
mod route;
mod router;
mod reader;
mod task;
mod staticfiles;
mod server;
mod wsframe;
mod wsproto;

pub mod ws;
pub mod dev;
pub mod httpcodes;
pub use error::ParseError;
pub use application::{Application, ApplicationBuilder};
pub use httprequest::HttpRequest;
pub use httpresponse::{Body, HttpResponse, HttpResponseBuilder};
pub use payload::{Payload, PayloadItem, PayloadError};
pub use router::{Router, RoutingMap};
pub use route::{Route, RouteFactory, RouteHandler};
pub use resource::{Reply, Resource};
pub use recognizer::{Params, RouteRecognizer};
pub use server::HttpServer;
pub use context::HttpContext;
pub use staticfiles::StaticFiles;

// re-exports
pub use http::{Method, StatusCode};
pub use cookie::{Cookie, CookieBuilder};
pub use cookie::{ParseError as CookieParseError};
pub use http_range::{HttpRange, HttpRangeParseError};

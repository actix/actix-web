//! Http framework for [Actix](https://github.com/fafhrd91/actix)

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
extern crate mime;
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
mod logger;
mod payload;
mod resource;
mod recognizer;
mod route;
mod reader;
mod task;
mod staticfiles;
mod server;
mod wsframe;
mod wsproto;

pub mod ws;
pub mod dev;
pub mod httpcodes;
pub mod multipart;
pub use error::ParseError;
pub use application::{Application, ApplicationBuilder, Middleware};
pub use httprequest::{HttpRequest, UrlEncoded};
pub use httpresponse::{Body, HttpResponse, HttpResponseBuilder};
pub use payload::{Payload, PayloadItem, PayloadError};
pub use route::{Route, RouteFactory, RouteHandler, RouteResult};
pub use resource::{Reply, Resource};
pub use recognizer::{Params, RouteRecognizer};
pub use logger::Logger;
pub use server::HttpServer;
pub use context::HttpContext;
pub use staticfiles::StaticFiles;

// re-exports
pub use http::{Method, StatusCode};
pub use cookie::{Cookie, CookieBuilder};
pub use cookie::{ParseError as CookieParseError};
pub use http_range::{HttpRange, HttpRangeParseError};

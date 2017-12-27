//! Actix web is a small, fast, down-to-earth, open source rust web framework.
//!
//! ```rust,ignore
//! use actix_web::*;
//!
//! fn index(req: HttpRequest) -> String {
//!     format!("Hello {}!", &req.match_info()["name"])
//! }
//!
//! fn main() {
//!     HttpServer::new(
//!         || Application::new()
//!             .resource("/{name}", |r| r.f(index)))
//!         .bind("127.0.0.1:8080")?
//!         .start()
//! }
//! ```
//!
//! ## Documentation
//!
//! * [User Guide](http://actix.github.io/actix-web/guide/)
//! * Cargo package: [actix-web](https://crates.io/crates/actix-web)
//! * Minimum supported Rust version: 1.20 or later
//!
//! ## Features
//!
//! * Supported *HTTP/1.x* and *HTTP/2.0* protocols
//! * Streaming and pipelining
//! * Keep-alive and slow requests handling
//! * `WebSockets`
//! * Transparent content compression/decompression (br, gzip, deflate)
//! * Configurable request routing
//! * Multipart streams
//! * Middlewares (`Logger`, `Session`, `DefaultHeaders`)
//! * Built on top of [Actix](https://github.com/actix/actix).

#![cfg_attr(actix_nightly, feature(
    specialization, // for impl ErrorResponse for std::error::Error
))]

#[macro_use]
extern crate log;
extern crate time;
extern crate bytes;
extern crate sha1;
extern crate regex;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate futures;
extern crate tokio_io;
extern crate tokio_core;
extern crate mio;
extern crate net2;

extern crate failure;
#[macro_use] extern crate failure_derive;

extern crate cookie;
extern crate http;
extern crate httparse;
extern crate http_range;
extern crate mime;
extern crate mime_guess;
extern crate url;
extern crate libc;
extern crate serde;
extern crate serde_json;
extern crate flate2;
extern crate brotli2;
extern crate percent_encoding;
extern crate smallvec;
extern crate num_cpus;
extern crate h2 as http2;
#[macro_use] extern crate actix;

#[cfg(test)]
#[macro_use] extern crate serde_derive;

#[cfg(feature="tls")]
extern crate native_tls;
#[cfg(feature="tls")]
extern crate tokio_tls;

#[cfg(feature="openssl")]
extern crate openssl;
#[cfg(feature="openssl")]
extern crate tokio_openssl;

mod application;
mod body;
mod context;
mod helpers;
mod encoding;
mod httprequest;
mod httpresponse;
mod info;
mod json;
mod route;
mod router;
mod param;
mod resource;
mod handler;
mod pipeline;
mod server;
mod channel;
mod wsframe;
mod wsproto;
mod h1;
mod h2;
mod h1writer;
mod h2writer;

pub mod fs;
pub mod ws;
pub mod error;
pub mod httpcodes;
pub mod multipart;
pub mod middleware;
pub mod pred;
pub mod test;
pub mod payload;
pub use error::{Error, Result, ResponseError};
pub use body::{Body, Binary};
pub use json::{Json};
pub use application::Application;
pub use httprequest::HttpRequest;
pub use httpresponse::HttpResponse;
pub use handler::{Reply, Responder, NormalizePath, AsyncResponder};
pub use route::Route;
pub use resource::Resource;
pub use server::HttpServer;
pub use context::HttpContext;

// re-exports
pub use http::{Method, StatusCode, Version};

#[doc(hidden)]
#[cfg(feature="tls")]
pub use native_tls::Pkcs12;

#[doc(hidden)]
#[cfg(feature="openssl")]
pub use openssl::pkcs12::Pkcs12;

pub mod headers {
//! Headers implementation

    pub use encoding::ContentEncoding;
    pub use httpresponse::ConnectionType;

    pub use cookie::Cookie;
    pub use cookie::CookieBuilder;
    pub use http_range::HttpRange;
}

pub mod dev {
//! The `actix-web` prelude for library developers
//!
//! The purpose of this module is to alleviate imports of many common actix traits
//! by adding a glob import to the top of actix heavy modules:
//!
//! ```
//! # #![allow(unused_imports)]
//! use actix_web::dev::*;
//! ```

    pub use body::BodyStream;
    pub use info::ConnectionInfo;
    pub use handler::Handler;
    pub use json::JsonBody;
    pub use router::{Router, Pattern};
    pub use pipeline::Pipeline;
    pub use channel::{HttpChannel, HttpHandler, IntoHttpHandler};
    pub use param::{FromParam, Params};
    pub use httprequest::UrlEncoded;
    pub use httpresponse::HttpResponseBuilder;

    pub use server::{ServerSettings, PauseServer, ResumeServer, StopServer};
}

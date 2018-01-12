//! Actix web is a small, fast, pragmatic, open source rust web framework.
//!
//! ```rust
//! use actix_web::*;
//! # use std::thread;
//!
//! fn index(req: HttpRequest) -> String {
//!     format!("Hello {}!", &req.match_info()["name"])
//! }
//!
//! fn main() {
//! # thread::spawn(|| {
//!     HttpServer::new(
//!         || Application::new()
//!             .resource("/{name}", |r| r.f(index)))
//!         .bind("127.0.0.1:8080").unwrap()
//!         .run();
//! # });
//! }
//! ```
//!
//! ## Documentation
//!
//! * [User Guide](http://actix.github.io/actix-web/guide/)
//! * Cargo package: [actix-web](https://crates.io/crates/actix-web)
//! * [GitHub repository](https://github.com/actix/actix-web)
//! * Supported Rust version: 1.20 or later
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
//! * Graceful server shutdown
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
extern crate failure;
#[macro_use]
extern crate futures;
extern crate tokio_io;
extern crate tokio_core;
extern crate mio;
extern crate net2;
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

pub mod fs;
pub mod ws;
pub mod error;
pub mod httpcodes;
pub mod multipart;
pub mod middleware;
pub mod pred;
pub mod test;
pub mod payload;
pub mod server;
pub use error::{Error, Result, ResponseError};
pub use body::{Body, Binary};
pub use json::Json;
pub use application::Application;
pub use httprequest::HttpRequest;
pub use httpresponse::HttpResponse;
pub use handler::{Reply, Responder, NormalizePath, AsyncResponder};
pub use route::Route;
pub use resource::Resource;
pub use context::HttpContext;
pub use server::HttpServer;

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

    pub use httpresponse::ConnectionType;

    pub use cookie::{Cookie, CookieBuilder};
    pub use http_range::HttpRange;

    /// Represents supported types of content encodings
    #[derive(Copy, Clone, PartialEq, Debug)]
    pub enum ContentEncoding {
        /// Automatically select encoding based on encoding negotiation
        Auto,
        /// A format using the Brotli algorithm
        Br,
        /// A format using the zlib structure with deflate algorithm
        Deflate,
        /// Gzip algorithm
        Gzip,
        /// Indicates the identity function (i.e. no compression, nor modification)
        Identity,
    }
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
    pub use param::{FromParam, Params};
    pub use httprequest::{UrlEncoded, RequestBody};
    pub use httpresponse::HttpResponseBuilder;
}

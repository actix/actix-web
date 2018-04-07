//! Actix web is a small, pragmatic, and extremely fast web framework
//! for Rust.
//!
//! ```rust
//! use actix_web::{server, App, Path};
//! # use std::thread;
//!
//! fn index(info: Path<(String, u32)>) -> String {
//!    format!("Hello {}! id:{}", info.0, info.1)
//! }
//!
//! fn main() {
//! # thread::spawn(|| {
//!     server::new(
//!         || App::new()
//!             .resource("/{name}/{id}/index.html", |r| r.with(index)))
//!         .bind("127.0.0.1:8080").unwrap()
//!         .run();
//! # });
//! }
//! ```
//!
//! ## Documentation & community resources
//!
//! Besides the API documentation (which you are currently looking
//! at!), several other resources are available:
//!
//! * [User Guide](http://actix.github.io/actix-web/guide/)
//! * [Chat on gitter](https://gitter.im/actix/actix)
//! * [GitHub repository](https://github.com/actix/actix-web)
//! * [Cargo package](https://crates.io/crates/actix-web)
//!
//! To get started navigating the API documentation you may want to
//! consider looking at the following pages:
//!
//! * [App](struct.App.html): This struct represents an actix-web
//!   application and is used to configure routes and other common
//!   settings.
//!
//! * [HttpServer](server/struct.HttpServer.html): This struct
//!   represents an HTTP server instance and is used to instantiate and
//!   configure servers.
//!
//! * [HttpRequest](struct.HttpRequest.html) and
//!   [HttpResponse](struct.HttpResponse.html): These structs
//!   represent HTTP requests and responses and expose various methods
//!   for inspecting, creating and otherwise utilising them.
//!
//! ## Features
//!
//! * Supported *HTTP/1.x* and *HTTP/2.0* protocols
//! * Streaming and pipelining
//! * Keep-alive and slow requests handling
//! * `WebSockets` server/client
//! * Transparent content compression/decompression (br, gzip, deflate)
//! * Configurable request routing
//! * Graceful server shutdown
//! * Multipart streams
//! * SSL support with OpenSSL or `native-tls`
//! * Middlewares (`Logger`, `Session`, `CORS`, `CSRF`, `DefaultHeaders`)
//! * Built on top of [Actix actor framework](https://github.com/actix/actix)
//! * Supported Rust version: 1.21 or later

#![cfg_attr(actix_nightly, feature(
    specialization, // for impl ErrorResponse for std::error::Error
))]
#![cfg_attr(feature = "cargo-clippy", allow(
    decimal_literal_representation,suspicious_arithmetic_impl,))]

#[macro_use]
extern crate log;
extern crate time;
extern crate base64;
extern crate bytes;
extern crate byteorder;
extern crate sha1;
extern crate regex;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate futures;
extern crate futures_cpupool;
extern crate tokio_io;
extern crate tokio_core;
extern crate mio;
extern crate net2;
extern crate cookie;
extern crate http as modhttp;
extern crate httparse;
extern crate http_range;
extern crate mime;
extern crate mime_guess;
extern crate language_tags;
extern crate rand;
extern crate url;
extern crate libc;
#[macro_use] extern crate serde;
extern crate serde_json;
extern crate serde_urlencoded;
extern crate flate2;
#[cfg(feature="brotli")]
extern crate brotli2;
extern crate encoding;
extern crate percent_encoding;
extern crate smallvec;
extern crate num_cpus;
extern crate h2 as http2;
extern crate trust_dns_resolver;
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
mod de;
mod extractor;
mod handler;
mod header;
mod helpers;
mod httpmessage;
mod httprequest;
mod httpresponse;
mod info;
mod json;
mod route;
mod router;
mod resource;
mod param;
mod payload;
mod pipeline;
mod with;

pub mod client;
pub mod fs;
pub mod ws;
pub mod error;
pub mod multipart;
pub mod middleware;
pub mod pred;
pub mod test;
pub mod server;
pub use extractor::{Path, Form, Query};
pub use error::{Error, Result, ResponseError};
pub use body::{Body, Binary};
pub use json::Json;
pub use application::App;
pub use httpmessage::HttpMessage;
pub use httprequest::HttpRequest;
pub use httpresponse::HttpResponse;
pub use handler::{Either, Responder, AsyncResponder, FromRequest, FutureResponse, State};
pub use context::HttpContext;

#[doc(hidden)]
pub mod httpcodes;

#[doc(hidden)]
#[allow(deprecated)]
pub use application::Application;

#[cfg(feature="openssl")]
pub(crate) const HAS_OPENSSL: bool = true;
#[cfg(not(feature="openssl"))]
pub(crate) const HAS_OPENSSL: bool = false;

#[cfg(feature="tls")]
pub(crate) const HAS_TLS: bool = true;
#[cfg(not(feature="tls"))]
pub(crate) const HAS_TLS: bool = false;

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
    pub use context::Drain;
    pub use json::{JsonBody, JsonConfig};
    pub use info::ConnectionInfo;
    pub use handler::{Handler, Reply};
    pub use extractor::{FormConfig, PayloadConfig};
    pub use route::Route;
    pub use router::{Router, Resource, ResourceType};
    pub use resource::ResourceHandler;
    pub use param::{FromParam, Params};
    pub use httpmessage::{UrlEncoded, MessageBody};
    pub use httpresponse::HttpResponseBuilder;
}

pub mod http {
    //! Various HTTP related types

    // re-exports
    pub use modhttp::{Method, StatusCode, Version};

    #[doc(hidden)]
    pub use modhttp::{uri, Uri, Error, Extensions, HeaderMap, HttpTryFrom};

    pub use http_range::HttpRange;
    pub use cookie::{Cookie, CookieBuilder};

    pub use helpers::NormalizePath;

    pub mod header {
        pub use ::header::*;
    }
    pub use header::ContentEncoding;
    pub use httpresponse::ConnectionType;
}

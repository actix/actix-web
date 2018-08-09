//! Actix web is a small, pragmatic, and extremely fast web framework
//! for Rust.
//!
//! ```rust
//! use actix_web::{server, App, Path, Responder};
//! # use std::thread;
//!
//! fn index(info: Path<(String, u32)>) -> impl Responder {
//!     format!("Hello {}! id:{}", info.0, info.1)
//! }
//!
//! fn main() {
//!     # thread::spawn(|| {
//!     server::new(|| {
//!         App::new().resource("/{name}/{id}/index.html", |r| r.with(index))
//!     }).bind("127.0.0.1:8080")
//!         .unwrap()
//!         .run();
//!     # });
//! }
//! ```
//!
//! ## Documentation & community resources
//!
//! Besides the API documentation (which you are currently looking
//! at!), several other resources are available:
//!
//! * [User Guide](https://actix.rs/docs/)
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
//!   for inspecting, creating and otherwise utilizing them.
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
//! * Supported Rust version: 1.26 or later
//!
//! ## Package feature
//!
//! * `tls` - enables ssl support via `native-tls` crate
//! * `alpn` - enables ssl support via `openssl` crate, require for `http/2`
//!    support
//! * `session` - enables session support, includes `ring` crate as
//!   dependency
//! * `brotli` - enables `brotli` compression support, requires `c`
//!   compiler
//! * `flate2-c` - enables `gzip`, `deflate` compression support, requires
//!   `c` compiler
//! * `flate2-rust` - experimental rust based implementation for
//!   `gzip`, `deflate` compression.
//!
#![cfg_attr(actix_nightly, feature(
    specialization, // for impl ErrorResponse for std::error::Error
    extern_prelude,
))]
#![cfg_attr(
    feature = "cargo-clippy",
    allow(decimal_literal_representation, suspicious_arithmetic_impl)
)]
#![warn(missing_docs)]

#[macro_use]
extern crate log;
extern crate base64;
extern crate byteorder;
extern crate bytes;
extern crate regex;
extern crate sha1;
extern crate time;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate futures;
extern crate cookie;
extern crate futures_cpupool;
extern crate htmlescape;
extern crate http as modhttp;
extern crate httparse;
extern crate language_tags;
extern crate lazycell;
extern crate mime;
extern crate mime_guess;
extern crate mio;
extern crate net2;
extern crate parking_lot;
extern crate rand;
extern crate slab;
extern crate tokio;
extern crate tokio_io;
extern crate tokio_reactor;
extern crate tokio_tcp;
extern crate tokio_timer;
extern crate url;
#[macro_use]
extern crate serde;
#[cfg(feature = "brotli")]
extern crate brotli2;
extern crate encoding;
#[cfg(feature = "flate2")]
extern crate flate2;
extern crate h2 as http2;
extern crate num_cpus;
#[macro_use]
extern crate percent_encoding;
extern crate serde_json;
extern crate smallvec;
#[macro_use]
extern crate actix as actix_inner;

#[cfg(test)]
#[macro_use]
extern crate serde_derive;

#[cfg(feature = "tls")]
extern crate native_tls;

#[cfg(feature = "openssl")]
extern crate openssl;
#[cfg(feature = "openssl")]
extern crate tokio_openssl;

#[cfg(feature = "rust-tls")]
extern crate rustls;
#[cfg(feature = "rust-tls")]
extern crate tokio_rustls;
#[cfg(feature = "rust-tls")]
extern crate webpki;
#[cfg(feature = "rust-tls")]
extern crate webpki_roots;

mod application;
mod body;
mod context;
mod de;
mod extensions;
mod extractor;
mod handler;
mod header;
mod helpers;
mod httpcodes;
mod httpmessage;
mod httprequest;
mod httpresponse;
mod info;
mod json;
mod param;
mod payload;
mod pipeline;
mod resource;
mod route;
mod router;
mod scope;
mod serde_urlencoded;
mod uri;
mod with;

pub mod client;
pub mod error;
pub mod fs;
pub mod middleware;
pub mod multipart;
pub mod pred;
pub mod server;
pub mod test;
pub mod ws;
pub use application::App;
pub use body::{Binary, Body};
pub use context::HttpContext;
pub use error::{Error, ResponseError, Result};
pub use extensions::Extensions;
pub use extractor::{Form, Path, Query};
pub use handler::{
    AsyncResponder, Either, FromRequest, FutureResponse, Responder, State,
};
pub use httpmessage::HttpMessage;
pub use httprequest::HttpRequest;
pub use httpresponse::HttpResponse;
pub use json::Json;
pub use scope::Scope;
pub use server::Request;

pub mod actix {
    //! Re-exports [actix's](https://docs.rs/actix/) prelude

    extern crate actix;
    pub use self::actix::actors::resolver;
    pub use self::actix::actors::signal;
    pub use self::actix::fut;
    pub use self::actix::msgs;
    pub use self::actix::prelude::*;
    pub use self::actix::{run, spawn};
}

#[cfg(feature = "openssl")]
pub(crate) const HAS_OPENSSL: bool = true;
#[cfg(not(feature = "openssl"))]
pub(crate) const HAS_OPENSSL: bool = false;

#[cfg(feature = "tls")]
pub(crate) const HAS_TLS: bool = true;
#[cfg(not(feature = "tls"))]
pub(crate) const HAS_TLS: bool = false;

#[cfg(feature = "rust-tls")]
pub(crate) const HAS_RUSTLS: bool = true;
#[cfg(not(feature = "rust-tls"))]
pub(crate) const HAS_RUSTLS: bool = false;

pub mod dev {
    //! The `actix-web` prelude for library developers
    //!
    //! The purpose of this module is to alleviate imports of many common actix
    //! traits by adding a glob import to the top of actix heavy modules:
    //!
    //! ```
    //! # #![allow(unused_imports)]
    //! use actix_web::dev::*;
    //! ```

    pub use body::BodyStream;
    pub use context::Drain;
    pub use extractor::{FormConfig, PayloadConfig};
    pub use handler::{AsyncResult, Handler};
    pub use httpmessage::{MessageBody, UrlEncoded};
    pub use httpresponse::HttpResponseBuilder;
    pub use info::ConnectionInfo;
    pub use json::{JsonBody, JsonConfig};
    pub use param::{FromParam, Params};
    pub use payload::{Payload, PayloadBuffer};
    pub use resource::Resource;
    pub use route::Route;
    pub use router::{ResourceDef, ResourceInfo, ResourceType, Router};
}

pub mod http {
    //! Various HTTP related types

    // re-exports
    pub use modhttp::{Method, StatusCode, Version};

    #[doc(hidden)]
    pub use modhttp::{uri, Error, Extensions, HeaderMap, HttpTryFrom, Uri};

    pub use cookie::{Cookie, CookieBuilder};

    pub use helpers::NormalizePath;

    /// Various http headers
    pub mod header {
        pub use header::*;
        pub use header::{ContentDisposition, DispositionType, DispositionParam, Charset, LanguageTag};
    }
    pub use header::ContentEncoding;
    pub use httpresponse::ConnectionType;
}

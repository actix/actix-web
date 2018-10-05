//! Actix web is a small, pragmatic, and extremely fast web framework
//! for Rust.
//!
//! ```rust,ignore
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
//! * `ssl` - enables ssl support via `openssl` crate, supports `http/2`
//! * `rust-tls` - enables ssl support via `rustls` crate, supports `http/2`
//! * `uds` - enables support for making client requests via Unix Domain Sockets.
//!   Unix only. Not necessary for *serving* requests.
//! * `session` - enables session support, includes `ring` crate as
//!   dependency
//! * `brotli` - enables `brotli` compression support, requires `c`
//!   compiler
//! * `flate2-c` - enables `gzip`, `deflate` compression support, requires
//!   `c` compiler
//! * `flate2-rust` - experimental rust based implementation for
//!   `gzip`, `deflate` compression.
//!
#![cfg_attr(actix_nightly, feature(tool_lints))]
// #![warn(missing_docs)]
#![allow(dead_code)]

extern crate actix;
extern crate actix_net;
#[macro_use]
extern crate log;
extern crate base64;
extern crate byteorder;
extern crate bytes;
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
extern crate encoding;
extern crate http as modhttp;
extern crate httparse;
extern crate mime;
extern crate net2;
extern crate rand;
extern crate serde;
extern crate serde_json;
extern crate serde_urlencoded;
extern crate tokio;
extern crate tokio_codec;
extern crate tokio_current_thread;
extern crate tokio_io;
extern crate tokio_reactor;
extern crate tokio_tcp;
extern crate tokio_timer;
#[cfg(all(unix, feature = "uds"))]
extern crate tokio_uds;
extern crate url;

#[cfg(test)]
#[macro_use]
extern crate serde_derive;

mod body;
mod config;
mod extensions;
mod header;
mod httpcodes;
mod httpmessage;
mod httpresponse;
mod json;
mod payload;
mod request;
mod uri;

#[doc(hidden)]
pub mod framed;

pub mod error;
pub mod h1;
pub(crate) mod helpers;
pub mod test;
pub mod ws;
pub use body::{Binary, Body};
pub use error::{Error, ResponseError, Result};
pub use extensions::Extensions;
pub use httpmessage::HttpMessage;
pub use httpresponse::HttpResponse;
pub use json::Json;
pub use request::Request;

pub use self::config::{KeepAlive, ServiceConfig, ServiceConfigBuilder};

pub mod dev {
    //! The `actix-web` prelude for library developers
    //!
    //! The purpose of this module is to alleviate imports of many common actix
    //! traits by adding a glob import to the top of actix heavy modules:
    //!
    //! ```
    //! # #![allow(unused_imports)]
    //! use actix_http::dev::*;
    //! ```

    pub use body::BodyStream;
    pub use httpmessage::{MessageBody, Readlines, UrlEncoded};
    pub use httpresponse::HttpResponseBuilder;
    pub use json::JsonBody;
    pub use payload::{Payload, PayloadBuffer};
}

pub mod http {
    //! Various HTTP related types

    // re-exports
    pub use modhttp::{Method, StatusCode, Version};

    #[doc(hidden)]
    pub use modhttp::{uri, Error, Extensions, HeaderMap, HttpTryFrom, Uri};

    pub use cookie::{Cookie, CookieBuilder};

    /// Various http headers
    pub mod header {
        pub use header::*;
    }
    pub use header::ContentEncoding;
    pub use httpresponse::ConnectionType;
}

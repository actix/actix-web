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
//! * [Request](struct.Request.html) and
//!   [Response](struct.Response.html): These structs
//!   represent HTTP requests and responses and expose various methods
//!   for inspecting, creating and otherwise utilizing them.
//!
//! ## Features
//!
//! * Supported *HTTP/1.x* protocol
//! * Streaming and pipelining
//! * Keep-alive and slow requests handling
//! * `WebSockets` server/client
//! * Supported Rust version: 1.26 or later
//!
//! ## Package feature
//!
//! * `session` - enables session support, includes `ring` crate as
//!   dependency
//!
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
extern crate futures;
extern crate cookie;
extern crate encoding;
extern crate http as modhttp;
extern crate httparse;
extern crate indexmap;
extern crate mime;
extern crate net2;
extern crate percent_encoding;
extern crate rand;
extern crate serde;
extern crate serde_json;
extern crate serde_urlencoded;
extern crate slab;
extern crate tokio;
extern crate tokio_codec;
extern crate tokio_current_thread;
extern crate tokio_io;
extern crate tokio_tcp;
extern crate tokio_timer;
extern crate trust_dns_proto;
extern crate trust_dns_resolver;
extern crate url as urlcrate;

#[cfg(test)]
#[macro_use]
extern crate serde_derive;

#[cfg(feature = "ssl")]
extern crate openssl;

pub mod body;
pub mod client;
mod config;
mod extensions;
mod header;
mod httpcodes;
mod httpmessage;
mod json;
mod message;
mod payload;
mod request;
mod response;
mod service;

pub mod error;
pub mod h1;
pub(crate) mod helpers;
pub mod test;
pub mod ws;
pub use body::{Body, MessageBody};
pub use error::{Error, ResponseError, Result};
pub use extensions::Extensions;
pub use httpmessage::HttpMessage;
pub use request::Request;
pub use response::Response;
pub use service::{SendError, SendResponse};

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

    pub use httpmessage::{MessageBody, Readlines, UrlEncoded};
    pub use json::JsonBody;
    pub use payload::{Payload, PayloadBuffer};
    pub use response::ResponseBuilder;
}

pub mod http {
    //! Various HTTP related types

    // re-exports
    pub use modhttp::header::{HeaderName, HeaderValue};
    pub use modhttp::{Method, StatusCode, Version};

    #[doc(hidden)]
    pub use modhttp::{uri, Error, HeaderMap, HttpTryFrom, Uri};

    #[doc(hidden)]
    pub use modhttp::uri::PathAndQuery;

    pub use cookie::{Cookie, CookieBuilder};

    /// Various http headers
    pub mod header {
        pub use header::*;
    }
    pub use header::ContentEncoding;
    pub use message::ConnectionType;
}

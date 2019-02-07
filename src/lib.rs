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
#![allow(
    clippy::type_complexity,
    clippy::new_without_default,
    clippy::new_without_default_derive
)]

#[macro_use]
extern crate log;

pub mod body;
pub mod client;
mod config;
mod extensions;
mod header;
mod helpers;
mod httpcodes;
mod httpmessage;
mod json;
mod message;
mod request;
mod response;
mod service;

pub mod error;
pub mod h1;
pub mod h2;
pub mod payload;
pub mod test;
pub mod ws;

pub use self::body::{Body, MessageBody};
pub use self::config::{KeepAlive, ServiceConfig, ServiceConfigBuilder};
pub use self::error::{Error, ResponseError, Result};
pub use self::extensions::Extensions;
pub use self::httpmessage::HttpMessage;
pub use self::request::Request;
pub use self::response::Response;
pub use self::service::{SendError, SendResponse};

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

    pub use crate::httpmessage::{MessageBody, Readlines, UrlEncoded};
    pub use crate::json::JsonBody;
    pub use crate::response::ResponseBuilder;
}

pub mod http {
    //! Various HTTP related types

    // re-exports
    pub use http::header::{HeaderName, HeaderValue};
    pub use http::{Method, StatusCode, Version};

    #[doc(hidden)]
    pub use http::{uri, Error, HeaderMap, HttpTryFrom, Uri};

    #[doc(hidden)]
    pub use http::uri::PathAndQuery;

    pub use cookie::{Cookie, CookieBuilder};

    /// Various http headers
    pub mod header {
        pub use crate::header::*;
    }
    pub use crate::header::ContentEncoding;
    pub use crate::message::ConnectionType;
}

//! Actix web is a small, fast, down-to-earth, open source rust web framework.

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
extern crate actix;
extern crate h2 as http2;

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
mod date;
mod encoding;
mod httprequest;
mod httpresponse;
mod payload;
mod info;
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
pub mod middlewares;
pub mod pred;
pub use error::{Error, Result};
pub use body::{Body, Binary};
pub use application::Application;
pub use httprequest::HttpRequest;
pub use httpresponse::HttpResponse;
pub use payload::{Payload, PayloadItem};
pub use handler::{Reply, Json, FromRequest};
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

    pub use info::ConnectionInfo;
    pub use handler::Handler;
    pub use router::{Router, Pattern};
    pub use pipeline::Pipeline;
    pub use channel::{HttpChannel, HttpHandler, IntoHttpHandler};
    pub use param::{FromParam, Params};
    pub use server::ServerSettings;
    pub use httprequest::UrlEncoded;
    pub use httpresponse::HttpResponseBuilder;
}

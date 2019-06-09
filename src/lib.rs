//! Actix web is a small, pragmatic, and extremely fast web framework
//! for Rust.
//!
//! ```rust
//! use actix_web::{web, App, Responder, HttpServer};
//! # use std::thread;
//!
//! fn index(info: web::Path<(String, u32)>) -> impl Responder {
//!     format!("Hello {}! id:{}", info.0, info.1)
//! }
//!
//! fn main() -> std::io::Result<()> {
//!     # thread::spawn(|| {
//!     HttpServer::new(|| App::new().service(
//!         web::resource("/{name}/{id}/index.html").to(index))
//!     )
//!         .bind("127.0.0.1:8080")?
//!         .run()
//!     # });
//!     # Ok(())
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
//! * [HttpServer](struct.HttpServer.html): This struct
//!   represents an HTTP server instance and is used to instantiate and
//!   configure servers.
//!
//! * [web](web/index.html): This module
//!   provide essentials helper functions and types for application registration.
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
//! * Multipart streams
//! * SSL support with OpenSSL or `native-tls`
//! * Middlewares (`Logger`, `Session`, `CORS`, `CSRF`, `DefaultHeaders`)
//! * Supports [Actix actor framework](https://github.com/actix/actix)
//! * Supported Rust version: 1.31 or later
//!
//! ## Package feature
//!
//! * `client` - enables http client (default enabled)
//! * `ssl` - enables ssl support via `openssl` crate, supports `http/2`
//! * `rust-tls` - enables ssl support via `rustls` crate, supports `http/2`
//! * `secure-cookies` - enables secure cookies support, includes `ring` crate as
//!   dependency (default enabled)
//! * `brotli` - enables `brotli` compression support, requires `c`
//!   compiler (default enabled)
//! * `flate2-zlib` - enables `gzip`, `deflate` compression support, requires
//!   `c` compiler (default enabled)
//! * `flate2-rust` - experimental rust based implementation for
//!   `gzip`, `deflate` compression.
//!
#![allow(clippy::type_complexity, clippy::new_without_default)]

mod app;
mod app_service;
mod config;
mod data;
pub mod error;
mod extract;
pub mod guard;
mod handler;
mod info;
pub mod middleware;
mod request;
mod resource;
mod responder;
mod rmap;
mod route;
mod scope;
mod server;
mod service;
pub mod test;
mod types;
pub mod web;

#[allow(unused_imports)]
#[macro_use]
extern crate actix_web_codegen;

#[doc(hidden)]
pub use actix_web_codegen::*;

// re-export for convenience
pub use actix_http::Response as HttpResponse;
pub use actix_http::{body, cookie, http, Error, HttpMessage, ResponseError, Result};

pub use crate::app::App;
pub use crate::extract::FromRequest;
pub use crate::request::HttpRequest;
pub use crate::resource::Resource;
pub use crate::responder::{Either, Responder};
pub use crate::route::Route;
pub use crate::scope::Scope;
pub use crate::server::HttpServer;

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

    pub use crate::config::{AppConfig, AppService};
    pub use crate::info::ConnectionInfo;
    pub use crate::rmap::ResourceMap;
    pub use crate::service::{
        HttpServiceFactory, ServiceRequest, ServiceResponse, WebService,
    };
    pub use crate::types::form::UrlEncoded;
    pub use crate::types::json::JsonBody;
    pub use crate::types::readlines::Readlines;

    pub use actix_http::body::{Body, BodySize, MessageBody, ResponseBody, SizedStream};
    pub use actix_http::encoding::Decoder as Decompress;
    pub use actix_http::ResponseBuilder as HttpResponseBuilder;
    pub use actix_http::{
        Extensions, Payload, PayloadStream, RequestHead, ResponseHead,
    };
    pub use actix_router::{Path, ResourceDef, ResourcePath, Url};
    pub use actix_server::Server;
    pub use actix_service::{Service, Transform};

    pub(crate) fn insert_slash(path: &str) -> String {
        let mut path = path.to_owned();
        if !path.is_empty() && !path.starts_with('/') {
            path.insert(0, '/');
        };
        path
    }
}

#[cfg(feature = "client")]
pub mod client {
    //! An HTTP Client
    //!
    //! ```rust
    //! # use futures::future::{Future, lazy};
    //! use actix_rt::System;
    //! use actix_web::client::Client;
    //!
    //! fn main() {
    //!     System::new("test").block_on(lazy(|| {
    //!        let mut client = Client::default();
    //!
    //!        client.get("http://www.rust-lang.org") // <- Create request builder
    //!           .header("User-Agent", "Actix-web")
    //!           .send()                             // <- Send http request
    //!           .map_err(|_| ())
    //!           .and_then(|response| {              // <- server http response
    //!                println!("Response: {:?}", response);
    //!                Ok(())
    //!           })
    //!     }));
    //! }
    //! ```
    pub use awc::error::{
        ConnectError, InvalidUrl, PayloadError, SendRequestError, WsClientError,
    };
    pub use awc::{
        test, Client, ClientBuilder, ClientRequest, ClientResponse, Connector,
    };
}

//! Actix Web is a powerful, pragmatic, and extremely fast web framework for Rust.
//!
//! # Examples
//! ```no_run
//! use actix_web::{get, web, App, HttpServer, Responder};
//!
//! #[get("/hello/{name}")]
//! async fn greet(name: web::Path<String>) -> impl Responder {
//!     format!("Hello {}!", name)
//! }
//!
//! #[actix_web::main] // or #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     HttpServer::new(|| {
//!         App::new().service(greet)
//!     })
//!     .bind(("127.0.0.1", 8080))?
//!     .run()
//!     .await
//! }
//! ```
//!
//! # Documentation & Community Resources
//! In addition to this API documentation, several other resources are available:
//!
//! * [Website & User Guide](https://actix.rs/)
//! * [Examples Repository](https://github.com/actix/examples)
//! * [Community Chat on Discord](https://discord.gg/NWpN5mmg3x)
//!
//! To get started navigating the API docs, you may consider looking at the following pages first:
//!
//! * [`App`]: This struct represents an Actix Web application and is used to
//!   configure routes and other common application settings.
//!
//! * [`HttpServer`]: This struct represents an HTTP server instance and is
//!   used to instantiate and configure servers.
//!
//! * [`web`]: This module provides essential types for route registration as well as
//!   common utilities for request handlers.
//!
//! * [`HttpRequest`] and [`HttpResponse`]: These
//!   structs represent HTTP requests and responses and expose methods for creating, inspecting,
//!   and otherwise utilizing them.
//!
//! # Features
//! - Supports HTTP/1.x and HTTP/2
//! - Streaming and pipelining
//! - Powerful [request routing](https://actix.rs/docs/url-dispatch/) with optional macros
//! - Full [Tokio](https://tokio.rs) compatibility
//! - Keep-alive and slow requests handling
//! - Client/server [WebSockets](https://actix.rs/docs/websockets/) support
//! - Transparent content compression/decompression (br, gzip, deflate, zstd)
//! - Multipart streams
//! - Static assets
//! - SSL support using OpenSSL or Rustls
//! - Middlewares ([Logger, Session, CORS, etc](middleware))
//! - Integrates with the [`awc` HTTP client](https://docs.rs/awc/)
//! - Runs on stable Rust 1.54+
//!
//! # Crate Features
//! - `cookies` - cookies support (enabled by default)
//! - `macros` - routing and runtime macros (enabled by default)
//! - `compress-brotli` - brotli content encoding compression support (enabled by default)
//! - `compress-gzip` - gzip and deflate content encoding compression support (enabled by default)
//! - `compress-zstd` - zstd content encoding compression support (enabled by default)
//! - `openssl` - HTTPS support via `openssl` crate, supports `HTTP/2`
//! - `rustls` - HTTPS support via `rustls` crate, supports `HTTP/2`
//! - `secure-cookies` - secure cookies support

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![allow(clippy::uninlined_format_args)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod app;
mod app_service;
mod config;
mod data;
pub mod dev;
pub mod error;
mod extract;
pub mod guard;
mod handler;
mod helpers;
pub mod http;
mod info;
pub mod middleware;
mod redirect;
mod request;
mod request_data;
mod resource;
mod response;
mod rmap;
mod route;
pub mod rt;
mod scope;
mod server;
mod service;
pub mod test;
pub(crate) mod types;
pub mod web;

pub use crate::app::App;
#[doc(inline)]
pub use crate::error::Result;
pub use crate::error::{Error, ResponseError};
pub use crate::extract::FromRequest;
pub use crate::handler::Handler;
pub use crate::request::HttpRequest;
pub use crate::resource::Resource;
pub use crate::response::{CustomizeResponder, HttpResponse, HttpResponseBuilder, Responder};
pub use crate::route::Route;
pub use crate::scope::Scope;
pub use crate::server::HttpServer;
pub use crate::types::Either;

pub use actix_http::{body, HttpMessage};

#[cfg(feature = "cookies")]
#[cfg_attr(docsrs, doc(cfg(feature = "cookies")))]
#[doc(inline)]
pub use cookie;

macro_rules! codegen_reexport {
    ($name:ident) => {
        #[cfg(feature = "macros")]
        #[cfg_attr(docsrs, doc(cfg(feature = "macros")))]
        pub use actix_web_codegen::$name;
    };
}

codegen_reexport!(main);
codegen_reexport!(test);
codegen_reexport!(route);
codegen_reexport!(routes);
codegen_reexport!(head);
codegen_reexport!(get);
codegen_reexport!(post);
codegen_reexport!(patch);
codegen_reexport!(put);
codegen_reexport!(delete);
codegen_reexport!(trace);
codegen_reexport!(connect);
codegen_reexport!(options);

pub(crate) type BoxError = Box<dyn std::error::Error>;

//! Actix web is a powerful, pragmatic, and extremely fast web framework for Rust.
//!
//! ## Example
//!
//! ```rust,no_run
//! use actix_web::{get, web, App, HttpServer, Responder};
//!
//! #[get("/{id}/{name}/index.html")]
//! async fn index(web::Path((id, name)): web::Path<(u32, String)>) -> impl Responder {
//!     format!("Hello {}! id:{}", name, id)
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     HttpServer::new(|| App::new().service(index))
//!         .bind("127.0.0.1:8080")?
//!         .run()
//!         .await
//! }
//! ```
//!
//! ## Documentation & Community Resources
//!
//! In addition to this API documentation, several other resources are available:
//!
//! * [Website & User Guide](https://actix.rs/)
//! * [Examples Repository](https://github.com/actix/examples)
//! * [Community Chat on Gitter](https://gitter.im/actix/actix-web)
//!
//! To get started navigating the API docs, you may consider looking at the following pages first:
//!
//! * [App](struct.App.html): This struct represents an Actix web application and is used to
//!   configure routes and other common application settings.
//!
//! * [HttpServer](struct.HttpServer.html): This struct represents an HTTP server instance and is
//!   used to instantiate and configure servers.
//!
//! * [web](web/index.html): This module provides essential types for route registration as well as
//!   common utilities for request handlers.
//!
//! * [HttpRequest](struct.HttpRequest.html) and [HttpResponse](struct.HttpResponse.html): These
//!   structs represent HTTP requests and responses and expose methods for creating, inspecting,
//!   and otherwise utilizing them.
//!
//! ## Features
//!
//! * Supports *HTTP/1.x* and *HTTP/2*
//! * Streaming and pipelining
//! * Keep-alive and slow requests handling
//! * Client/server [WebSockets](https://actix.rs/docs/websockets/) support
//! * Transparent content compression/decompression (br, gzip, deflate)
//! * Powerful [request routing](https://actix.rs/docs/url-dispatch/)
//! * Multipart streams
//! * Static assets
//! * SSL support using OpenSSL or Rustls
//! * Middlewares ([Logger, Session, CORS, etc](https://actix.rs/docs/middleware/))
//! * Includes an async [HTTP client](https://actix.rs/actix-web/actix_web/client/index.html)
//! * Supports [Actix actor framework](https://github.com/actix/actix)
//! * Runs on stable Rust 1.42+
//!
//! ## Crate Features
//!
//! * `compress` - content encoding compression support (enabled by default)
//! * `openssl` - HTTPS support via `openssl` crate, supports `HTTP/2`
//! * `rustls` - HTTPS support via `rustls` crate, supports `HTTP/2`
//! * `secure-cookies` - secure cookies support

#![deny(rust_2018_idioms)]
#![allow(clippy::needless_doctest_main, clippy::type_complexity)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

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
mod request_data;
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

pub use actix_http::Response as HttpResponse;
pub use actix_http::{body, cookie, http, Error, HttpMessage, ResponseError, Result};
pub use actix_rt as rt;
pub use actix_web_codegen::*;

pub use crate::app::App;
pub use crate::extract::FromRequest;
pub use crate::request::HttpRequest;
pub use crate::resource::Resource;
pub use crate::responder::Responder;
pub use crate::route::Route;
pub use crate::scope::Scope;
pub use crate::server::HttpServer;
pub use crate::types::{Either, EitherExtractError};

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
    #[doc(hidden)]
    pub use crate::handler::Factory;
    pub use crate::info::ConnectionInfo;
    pub use crate::rmap::ResourceMap;
    pub use crate::service::{
        HttpServiceFactory, ServiceRequest, ServiceResponse, WebService,
    };

    pub use crate::types::form::UrlEncoded;
    pub use crate::types::json::JsonBody;
    pub use crate::types::readlines::Readlines;

    pub use actix_http::body::{Body, BodySize, MessageBody, ResponseBody, SizedStream};
    #[cfg(feature = "compress")]
    pub use actix_http::encoding::Decoder as Decompress;
    pub use actix_http::ResponseBuilder as HttpResponseBuilder;
    pub use actix_http::{
        Extensions, Payload, PayloadStream, RequestHead, ResponseHead,
    };
    pub use actix_router::{Path, ResourceDef, ResourcePath, Url};
    pub use actix_server::Server;
    pub use actix_service::{Service, Transform};

    pub(crate) fn insert_slash(mut patterns: Vec<String>) -> Vec<String> {
        for path in &mut patterns {
            if !path.is_empty() && !path.starts_with('/') {
                path.insert(0, '/');
            };
        }
        patterns
    }

    use crate::http::header::ContentEncoding;
    use actix_http::{Response, ResponseBuilder};

    struct Enc(ContentEncoding);

    /// Helper trait that allows to set specific encoding for response.
    pub trait BodyEncoding {
        /// Get content encoding
        fn get_encoding(&self) -> Option<ContentEncoding>;

        /// Set content encoding
        fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self;
    }

    impl BodyEncoding for ResponseBuilder {
        fn get_encoding(&self) -> Option<ContentEncoding> {
            if let Some(ref enc) = self.extensions().get::<Enc>() {
                Some(enc.0)
            } else {
                None
            }
        }

        fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
            self.extensions_mut().insert(Enc(encoding));
            self
        }
    }

    impl<B> BodyEncoding for Response<B> {
        fn get_encoding(&self) -> Option<ContentEncoding> {
            if let Some(ref enc) = self.extensions().get::<Enc>() {
                Some(enc.0)
            } else {
                None
            }
        }

        fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
            self.extensions_mut().insert(Enc(encoding));
            self
        }
    }
}

pub mod client {
    //! Actix web async HTTP client.
    //!
    //! ```rust
    //! use actix_web::client::Client;
    //!
    //! #[actix_web::main]
    //! async fn main() {
    //!    let mut client = Client::default();
    //!
    //!    // Create request builder and send request
    //!    let response = client.get("http://www.rust-lang.org")
    //!       .header("User-Agent", "actix-web/3.0")
    //!       .send()     // <- Send request
    //!       .await;     // <- Wait for response
    //!
    //!    println!("Response: {:?}", response);
    //! }
    //! ```

    pub use awc::error::*;
    pub use awc::{
        test, Client, ClientBuilder, ClientRequest, ClientResponse, Connector,
    };
}

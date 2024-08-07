//! Actix actors support for Actix Web.
//!
//! This crate is deprecated. Migrate to [`actix-ws`](https://crates.io/crates/actix-ws).
//!
//! # Examples
//!
//! ```no_run
//! use actix::{Actor, StreamHandler};
//! use actix_web::{get, web, App, Error, HttpRequest, HttpResponse, HttpServer};
//! use actix_web_actors::ws;
//!
//! /// Define Websocket actor
//! struct MyWs;
//!
//! impl Actor for MyWs {
//!     type Context = ws::WebsocketContext<Self>;
//! }
//!
//! /// Handler for ws::Message message
//! impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWs {
//!     fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
//!         match msg {
//!             Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
//!             Ok(ws::Message::Text(text)) => ctx.text(text),
//!             Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
//!             _ => (),
//!         }
//!     }
//! }
//!
//! #[get("/ws")]
//! async fn index(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
//!     ws::start(MyWs, &req, stream)
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     HttpServer::new(|| App::new().service(index))
//!         .bind(("127.0.0.1", 8080))?
//!         .run()
//!         .await
//! }
//! ```
//!
//! # Documentation & Community Resources
//! In addition to this API documentation, several other resources are available:
//!
//! * [Website & User Guide](https://actix.rs/)
//! * [Documentation for `actix_web`](actix_web)
//! * [Examples Repository](https://github.com/actix/examples)
//! * [Community Chat on Discord](https://discord.gg/NWpN5mmg3x)
//!
//! To get started navigating the API docs, you may consider looking at the following pages first:
//!
//! * [`ws`]: This module provides actor support for WebSockets.
//!
//! * [`HttpContext`]: This struct provides actor support for streaming HTTP responses.
//!

#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod context;
pub mod ws;

pub use self::context::HttpContext;

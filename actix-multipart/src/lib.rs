//! Multipart form support for Actix Web.
//!
//! # Examples
//!
//! ```no_run
//! use actix_web::{post, App, HttpServer, Responder};
//!
//! use actix_multipart::form::{json::Json as MpJson, tempfile::TempFile, MultipartForm};
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize)]
//! struct Metadata {
//!     name: String,
//! }
//!
//! #[derive(Debug, MultipartForm)]
//! struct UploadForm {
//!     #[multipart(limit = "100MB")]
//!     file: TempFile,
//!     json: MpJson<Metadata>,
//! }
//!
//! #[post("/videos")]
//! pub async fn post_video(MultipartForm(form): MultipartForm<UploadForm>) -> impl Responder {
//!     format!(
//!         "Uploaded file {}, with size: {}",
//!         form.json.name, form.file.size
//!     )
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     HttpServer::new(move || App::new().service(post_video))
//!         .bind(("127.0.0.1", 8080))?
//!         .run()
//!         .await
//! }
//! ```
//!
//! cURL request:
//!
//! ```sh
//! curl -v --request POST \
//!   --url http://localhost:8080/videos \
//!   -F 'json={"name": "Cargo.lock"};type=application/json' \
//!   -F file=@./Cargo.lock
//! ```

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![allow(clippy::borrow_interior_mutable_const)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

// This allows us to use the actix_multipart_derive within this crate's tests
#[cfg(test)]
extern crate self as actix_multipart;

mod error;
mod extractor;
pub(crate) mod field;
pub mod form;
pub(crate) mod payload;
pub(crate) mod safety;
mod server;
pub mod test;

pub use self::{error::MultipartError, field::Field, server::Multipart};

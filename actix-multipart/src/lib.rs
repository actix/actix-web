//! Multipart form support for Actix web.

#![deny(rust_2018_idioms)]
#![allow(clippy::borrow_interior_mutable_const)]
#![clippy::msrv = "1.46"]

mod error;
mod extractor;
mod server;

pub use self::error::MultipartError;
pub use self::server::{Field, Multipart};

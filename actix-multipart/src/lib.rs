//! Multipart form support for Actix Web.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![allow(clippy::borrow_interior_mutable_const)]

mod error;
mod extractor;
mod server;

pub use self::error::MultipartError;
pub use self::server::{Field, Multipart};

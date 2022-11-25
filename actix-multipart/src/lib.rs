//! Multipart form support for Actix Web.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![allow(clippy::borrow_interior_mutable_const)]
#![cfg_attr(docsrs, feature(doc_cfg))]

// This allows us to use the actix_multipart_derive within this crate's tests
#[cfg(test)]
extern crate self as actix_multipart;
extern crate tempfile_dep as tempfile;

mod error;
mod extractor;
mod server;

pub mod form;

pub use self::error::MultipartError;
pub use self::server::{Field, Multipart};

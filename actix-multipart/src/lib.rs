//! Multipart form support for Actix Web.

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
pub mod form;
mod server;
pub mod test;

pub use self::{
    error::MultipartError,
    server::{Field, Multipart},
    test::{
        create_form_data_payload_and_headers, create_form_data_payload_and_headers_with_boundary,
    },
};

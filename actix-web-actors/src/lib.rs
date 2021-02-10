//! Actix actors support for Actix Web.

#![deny(rust_2018_idioms)]
#![allow(clippy::borrow_interior_mutable_const)]

mod context;
pub mod ws;

pub use self::context::HttpContext;

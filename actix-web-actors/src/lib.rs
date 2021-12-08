//! Actix actors support for Actix Web.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]

mod context;
pub mod ws;

pub use self::context::HttpContext;

#![allow(clippy::borrow_interior_mutable_const)]
//! Actix actors integration for Actix web framework
mod context;
pub mod ws;

pub use self::context::HttpContext;

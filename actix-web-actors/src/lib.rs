//! Actix actors integration for Actix web framework

#![allow(clippy::borrow_interior_mutable_const)]

mod context;
pub mod ws;

pub use self::context::HttpContext;

//! Actix net - framework for the compisible network services for Rust.
//!
//! ## Package feature
//!
//! * `tls` - enables ssl support via `native-tls` crate
//! * `ssl` - enables ssl support via `openssl` crate
//! * `rust-tls` - enables ssl support via `rustls` crate
// #![warn(missing_docs)]

#![allow(
    clippy::declare_interior_mutable_const,
    clippy::borrow_interior_mutable_const
)]

mod cell;
pub mod cloneable;
pub mod connector;
pub mod counter;
pub mod either;
pub mod framed;
pub mod inflight;
pub mod keepalive;
pub mod resolver;
pub mod server;
pub mod ssl;
pub mod stream;
pub mod time;
pub mod timeout;

#[derive(Copy, Clone, Debug)]
pub enum Never {}

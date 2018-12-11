//! Actix utils - various helper services
mod cell;
pub mod cloneable;
pub mod counter;
pub mod either;
pub mod framed;
pub mod inflight;
pub mod keepalive;
pub mod stream;
pub mod time;
pub mod timeout;

#[derive(Copy, Clone, Debug)]
pub enum Never {}

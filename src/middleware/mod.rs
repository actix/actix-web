//! Middlewares
mod compress;
pub use self::compress::{BodyEncoding, Compress};

pub mod cors;
mod defaultheaders;
pub mod errhandlers;
mod logger;

pub use self::defaultheaders::DefaultHeaders;
pub use self::logger::Logger;

#[cfg(feature = "secure-cookies")]
pub mod identity;

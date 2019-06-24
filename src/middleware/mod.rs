//! Middlewares
mod compress;
pub use self::compress::{BodyEncoding, Compress};

mod defaultheaders;
pub mod errhandlers;
mod logger;
mod normalize;

pub use self::defaultheaders::DefaultHeaders;
pub use self::logger::Logger;
pub use self::normalize::NormalizePath;

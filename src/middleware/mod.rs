//! Middlewares
#[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
mod compress;
#[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
pub use self::compress::Compress;

#[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
mod decompress;
#[cfg(any(feature = "brotli", feature = "flate2-zlib", feature = "flate2-rust"))]
pub use self::decompress::Decompress;

pub mod cors;
mod defaultheaders;
pub mod errhandlers;
mod logger;

pub use self::defaultheaders::DefaultHeaders;
pub use self::logger::Logger;

#[cfg(feature = "cookies")]
pub mod identity;

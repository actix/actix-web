#[cfg(any(feature = "brotli", feature = "flate2"))]
mod compress;
#[cfg(any(feature = "brotli", feature = "flate2"))]
pub use self::compress::Compress;

mod defaultheaders;
mod errhandlers;
mod logger;

pub use self::defaultheaders::DefaultHeaders;
pub use self::errhandlers::{ErrorHandlerResponse, ErrorHandlers};
pub use self::logger::Logger;

// #[cfg(feature = "session")]
// pub use actix_session as session;

#[cfg(feature = "session")]
pub mod identity;

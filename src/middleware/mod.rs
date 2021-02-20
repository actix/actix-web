//! Commonly used middleware.

mod compat;
mod condition;
mod default_headers;
mod err_handlers;
mod logger;
mod normalize;

pub use self::compat::Compat;
pub use self::condition::Condition;
pub use self::default_headers::DefaultHeaders;
pub use self::err_handlers::{ErrorHandlerResponse, ErrorHandlers};
pub use self::logger::Logger;
pub use self::normalize::{NormalizePath, TrailingSlash};

#[cfg(feature = "compress")]
mod compress;
#[cfg(feature = "compress")]
pub use self::compress::Compress;

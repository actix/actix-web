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

#[cfg(feature = "deprecated")]
#[deprecated(
    since = "1.0.1",
    note = "please use `actix_cors` instead. support will be removed in actix-web 1.0.2"
)]
pub use actix_cors as cors;

#[cfg(feature = "deprecated")]
#[deprecated(
    since = "1.0.1",
    note = "please use `actix_identity` instead. support will be removed in actix-web 1.0.2"
)]
pub use actix_identity as identity;

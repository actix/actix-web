//! The `actix-web` prelude for library developers
//!
//! The purpose of this module is to alleviate imports of many common actix traits
//! by adding a glob import to the top of actix heavy modules:
//!
//! ```
//! # #![allow(unused_imports)]
//! use actix_web::dev::*;
//! ```

// dev specific
pub use handler::Handler;
pub use pipeline::Pipeline;
pub use channel::{HttpChannel, HttpHandler};
pub use recognizer::{FromParam, RouteRecognizer};

pub use application::ApplicationBuilder;
pub use httpresponse::HttpResponseBuilder;
pub use cookie::CookieBuilder;

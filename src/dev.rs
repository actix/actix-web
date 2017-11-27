//! The `actix-web` prelude for library developers
//!
//! The purpose of this module is to alleviate imports of many common actix traits
//! by adding a glob import to the top of actix heavy modules:
//!
//! ```
//! # #![allow(unused_imports)]
//! use actix_web::dev::*;
//! ```

pub use super::*;

// dev specific
pub use task::Task;
pub use pipeline::Pipeline;
pub use route::RouteFactory;
pub use recognizer::RouteRecognizer;
pub use channel::HttpChannel;

//! Various HTTP related types.

pub mod header;

// TODO: figure out how best to expose http::Error vs actix_http::Error
pub use actix_http::{uri, ConnectionType, Error, Method, StatusCode, Uri, Version};

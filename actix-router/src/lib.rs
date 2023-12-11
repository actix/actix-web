//! Resource path matching and router.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![allow(clippy::uninlined_format_args)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod de;
mod path;
mod pattern;
mod quoter;
mod resource;
mod resource_path;
mod router;

#[cfg(any(feature = "http", feature = "http-1"))]
mod url;

#[cfg(any(feature = "http", feature = "http-1"))]
pub use self::url::Url;
pub use self::{
    de::PathDeserializer,
    path::Path,
    pattern::{IntoPatterns, Patterns},
    quoter::Quoter,
    resource::ResourceDef,
    resource_path::{Resource, ResourcePath},
    router::{ResourceId, Router, RouterBuilder},
};

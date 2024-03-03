//! Resource path matching and router.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod de;
mod path;
mod pattern;
mod quoter;
mod regex_set;
mod resource;
mod resource_path;
mod router;

#[cfg(feature = "http")]
mod url;

#[cfg(feature = "http")]
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

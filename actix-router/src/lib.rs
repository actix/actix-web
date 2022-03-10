//! Resource path matching and router.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

mod de;
mod path;
mod pattern;
mod quoter;
mod resource;
mod resource_path;
mod router;

#[cfg(feature = "http")]
mod url;

pub use self::de::PathDeserializer;
pub use self::path::Path;
pub use self::pattern::{IntoPatterns, Patterns};
pub use self::quoter::Quoter;
pub use self::resource::ResourceDef;
pub use self::resource_path::{Resource, ResourcePath};
pub use self::router::{ResourceId, Router, RouterBuilder};

#[cfg(feature = "http")]
pub use self::url::Url;

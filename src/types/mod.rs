//! Common extractors and responders.

mod either;
mod form;
mod header;
mod json;
mod path;
mod payload;
mod query;
mod readlines;

pub use self::either::Either;
pub use self::form::{Form, FormConfig, UrlEncoded};
pub use self::header::Header;
pub use self::json::{Json, JsonBody, JsonConfig};
pub use self::path::{Path, PathConfig};
pub use self::payload::{Payload, PayloadConfig};
pub use self::query::{Query, QueryConfig};
pub use self::readlines::Readlines;

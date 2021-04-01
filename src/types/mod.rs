//! Common extractors and responders.

// TODO: review visibility
mod either;
pub(crate) mod form;
mod header;
pub(crate) mod json;
mod path;
pub(crate) mod payload;
mod query;
pub(crate) mod readlines;

pub use self::either::{Either, EitherExtractError};
pub use self::form::{Form, FormConfig};
pub use self::header::Header;
pub use self::json::{Json, JsonConfig};
pub use self::path::{Path, PathConfig};
pub use self::payload::{Payload, PayloadConfig};
pub use self::query::{Query, QueryConfig};
pub use self::readlines::Readlines;

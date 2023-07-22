//! Common extractors and responders.

mod bincode;
mod either;
mod form;
mod header;
mod json;
mod path;
mod payload;
mod query;
mod readlines;

#[cfg(feature = "bincode")]
pub use self::bincode::{Bincode, BincodeBody, BincodeConfig};
pub use self::{
    either::Either,
    form::{Form, FormConfig, UrlEncoded},
    header::Header,
    json::{Json, JsonBody, JsonConfig},
    path::{Path, PathConfig},
    payload::{Payload, PayloadConfig},
    query::{Query, QueryConfig},
    readlines::Readlines,
};

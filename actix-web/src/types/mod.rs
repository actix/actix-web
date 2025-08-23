//! Common extractors and responders.

mod either;
mod form;
mod header;
mod html;
mod json;
mod path;
mod payload;
mod query;
mod readlines;

pub use self::{
    either::Either,
    form::{Form, FormConfig, UrlEncoded},
    header::Header,
    html::Html,
    json::{Json, JsonBody, JsonConfig},
    path::{Path, PathConfig},
    payload::{Payload, PayloadConfig},
    query::{Query, QueryConfig},
    readlines::Readlines,
};

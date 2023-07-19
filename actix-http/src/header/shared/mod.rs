//! Originally taken from `hyper::header::shared`.

pub use language_tags::LanguageTag;

mod charset;
mod content_encoding;
mod extended;
mod http_date;
mod quality;
mod quality_item;

pub use self::{
    charset::Charset,
    content_encoding::ContentEncoding,
    extended::{parse_extended_value, ExtendedValue},
    http_date::HttpDate,
    quality::{q, Quality},
    quality_item::QualityItem,
};

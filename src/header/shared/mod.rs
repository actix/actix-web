//! Copied for `hyper::header::shared`;

pub use self::charset::Charset;
pub use self::encoding::Encoding;
pub use self::entity::EntityTag;
pub use self::httpdate::HttpDate;
pub use self::quality_item::{q, qitem, Quality, QualityItem};
pub use language_tags::LanguageTag;

mod charset;
mod encoding;
mod entity;
mod httpdate;
mod quality_item;

//! Copied for `hyper::header::shared`;

pub use self::charset::Charset;
pub use self::encoding::Encoding;
pub use self::entity::EntityTag;
pub use self::httpdate::HttpDate;
pub use language_tags::LanguageTag;
pub use self::quality_item::{Quality, QualityItem, qitem, q};

mod charset;
mod entity;
mod encoding;
mod httpdate;
mod quality_item;

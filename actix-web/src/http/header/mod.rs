//! A Collection of Header implementations for common HTTP Headers.
//!
//! ## Mime Types
//! Several header fields use MIME values for their contents. Keeping with the strongly-typed theme,
//! the [mime] crate is used in such headers as [`ContentType`] and [`Accept`].

use std::fmt;

// re-export from actix-http
// - header name / value types
// - relevant traits for converting to header name / value
// - all const header names
// - header map
// - the few typed headers from actix-http
// - header parsing utils
pub use actix_http::header::*;
use bytes::{Bytes, BytesMut};

mod accept;
mod accept_charset;
mod accept_encoding;
mod accept_language;
mod allow;
mod cache_control;
mod content_disposition;
mod content_language;
mod content_length;
mod content_range;
mod content_type;
mod date;
mod encoding;
mod entity;
mod etag;
mod expires;
mod if_match;
mod if_modified_since;
mod if_none_match;
mod if_range;
mod if_unmodified_since;
mod last_modified;
mod macros;
mod preference;
mod range;

#[cfg(test)]
pub(crate) use self::macros::common_header_test;
pub(crate) use self::macros::{common_header, common_header_test_module};
pub use self::{
    accept::Accept,
    accept_charset::AcceptCharset,
    accept_encoding::AcceptEncoding,
    accept_language::AcceptLanguage,
    allow::Allow,
    cache_control::{CacheControl, CacheDirective},
    content_disposition::{ContentDisposition, DispositionParam, DispositionType},
    content_language::ContentLanguage,
    content_length::ContentLength,
    content_range::{ContentRange, ContentRangeSpec},
    content_type::ContentType,
    date::Date,
    encoding::Encoding,
    entity::EntityTag,
    etag::ETag,
    expires::Expires,
    if_match::IfMatch,
    if_modified_since::IfModifiedSince,
    if_none_match::IfNoneMatch,
    if_range::IfRange,
    if_unmodified_since::IfUnmodifiedSince,
    last_modified::LastModified,
    preference::Preference,
    range::{ByteRangeSpec, Range},
};

/// Format writer ([`fmt::Write`]) for a [`BytesMut`].
#[derive(Debug, Default)]
struct Writer {
    buf: BytesMut,
}

impl Writer {
    /// Constructs new bytes writer.
    pub fn new() -> Writer {
        Writer::default()
    }

    /// Splits bytes out of writer, leaving writer buffer empty.
    pub fn take(&mut self) -> Bytes {
        self.buf.split().freeze()
    }
}

impl fmt::Write for Writer {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }

    #[inline]
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        fmt::write(self, args)
    }
}

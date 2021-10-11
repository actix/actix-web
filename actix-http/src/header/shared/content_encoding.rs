use std::{convert::TryFrom, str::FromStr};

use derive_more::{Display, Error};
use http::header::InvalidHeaderValue;

use crate::{
    error::ParseError,
    header::{self, from_one_raw_str, Header, HeaderName, HeaderValue, IntoHeaderValue},
    HttpMessage,
};

/// Error return when a content encoding is unknown.
///
/// Example: 'compress'
#[derive(Debug, Display, Error)]
#[display(fmt = "unsupported content encoding")]
pub struct ContentEncodingParseError;

/// Represents a supported content encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation.
    Auto,

    /// A format using the Brotli algorithm.
    Br,

    /// A format using the zlib structure with deflate algorithm.
    Deflate,

    /// Gzip algorithm.
    Gzip,

    // Zstd algorithm.
    Zstd,

    /// Indicates the identity function (i.e. no compression, nor modification).
    Identity,
}

impl ContentEncoding {
    /// Is the content compressed?
    #[inline]
    pub fn is_compression(self) -> bool {
        matches!(self, ContentEncoding::Identity | ContentEncoding::Auto)
    }

    /// Convert content encoding to string.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Zstd => "zstd",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }
}

impl Default for ContentEncoding {
    fn default() -> Self {
        Self::Identity
    }
}

impl FromStr for ContentEncoding {
    type Err = ContentEncodingParseError;

    fn from_str(val: &str) -> Result<Self, Self::Err> {
        let val = val.trim();

        if val.eq_ignore_ascii_case("br") {
            Ok(ContentEncoding::Br)
        } else if val.eq_ignore_ascii_case("gzip") {
            Ok(ContentEncoding::Gzip)
        } else if val.eq_ignore_ascii_case("deflate") {
            Ok(ContentEncoding::Deflate)
        } else if val.eq_ignore_ascii_case("zstd") {
            Ok(ContentEncoding::Zstd)
        } else {
            Err(ContentEncodingParseError)
        }
    }
}

impl TryFrom<&str> for ContentEncoding {
    type Error = ContentEncodingParseError;

    fn try_from(val: &str) -> Result<Self, Self::Error> {
        val.parse()
    }
}

impl IntoHeaderValue for ContentEncoding {
    type Error = InvalidHeaderValue;

    fn try_into_value(self) -> Result<http::HeaderValue, Self::Error> {
        Ok(HeaderValue::from_static(self.as_str()))
    }
}

impl Header for ContentEncoding {
    fn name() -> HeaderName {
        header::CONTENT_ENCODING
    }

    fn parse<T: HttpMessage>(msg: &T) -> Result<Self, ParseError> {
        from_one_raw_str(msg.headers().get(Self::name()))
    }
}

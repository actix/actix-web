use std::str::FromStr;

use derive_more::{Display, Error};
use http::header::InvalidHeaderValue;

use crate::{
    error::ParseError,
    header::{self, from_one_raw_str, Header, HeaderName, HeaderValue, TryIntoHeaderValue},
    HttpMessage,
};

/// Error returned when a content encoding is unknown.
#[derive(Debug, Display, Error)]
#[display(fmt = "unsupported content encoding")]
pub struct ContentEncodingParseError;

/// Represents a supported content encoding.
///
/// Includes a commonly-used subset of media types appropriate for use as HTTP content encodings.
/// See [IANA HTTP Content Coding Registry].
///
/// [IANA HTTP Content Coding Registry]: https://www.iana.org/assignments/http-parameters/http-parameters.xhtml
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContentEncoding {
    /// Indicates the no-op identity encoding.
    ///
    /// I.e., no compression or modification.
    Identity,

    /// A format using the Brotli algorithm.
    Brotli,

    /// A format using the zlib structure with deflate algorithm.
    Deflate,

    /// Gzip algorithm.
    Gzip,

    /// Zstd algorithm.
    Zstd,
}

impl ContentEncoding {
    /// Convert content encoding to string.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            ContentEncoding::Brotli => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Zstd => "zstd",
            ContentEncoding::Identity => "identity",
        }
    }

    /// Convert content encoding to header value.
    #[inline]
    pub const fn to_header_value(self) -> HeaderValue {
        match self {
            ContentEncoding::Brotli => HeaderValue::from_static("br"),
            ContentEncoding::Gzip => HeaderValue::from_static("gzip"),
            ContentEncoding::Deflate => HeaderValue::from_static("deflate"),
            ContentEncoding::Zstd => HeaderValue::from_static("zstd"),
            ContentEncoding::Identity => HeaderValue::from_static("identity"),
        }
    }
}

impl Default for ContentEncoding {
    #[inline]
    fn default() -> Self {
        Self::Identity
    }
}

impl FromStr for ContentEncoding {
    type Err = ContentEncodingParseError;

    fn from_str(enc: &str) -> Result<Self, Self::Err> {
        let enc = enc.trim();

        if enc.eq_ignore_ascii_case("br") {
            Ok(ContentEncoding::Brotli)
        } else if enc.eq_ignore_ascii_case("gzip") {
            Ok(ContentEncoding::Gzip)
        } else if enc.eq_ignore_ascii_case("deflate") {
            Ok(ContentEncoding::Deflate)
        } else if enc.eq_ignore_ascii_case("identity") {
            Ok(ContentEncoding::Identity)
        } else if enc.eq_ignore_ascii_case("zstd") {
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

impl TryIntoHeaderValue for ContentEncoding {
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

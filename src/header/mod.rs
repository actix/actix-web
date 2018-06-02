//! Various http headers
// This is mostly copy of [hyper](https://github.com/hyperium/hyper/tree/master/src/header)

use std::fmt;
use std::str::FromStr;

use bytes::{Bytes, BytesMut};
use mime::Mime;
use modhttp::header::GetAll;
use modhttp::Error as HttpError;

pub use modhttp::header::*;

use error::ParseError;
use httpmessage::HttpMessage;

mod common;
mod shared;
#[doc(hidden)]
pub use self::common::*;
#[doc(hidden)]
pub use self::shared::*;

#[doc(hidden)]
/// A trait for any object that will represent a header field and value.
pub trait Header
where
    Self: IntoHeaderValue,
{
    /// Returns the name of the header field
    fn name() -> HeaderName;

    /// Parse a header
    fn parse<T: HttpMessage>(msg: &T) -> Result<Self, ParseError>;
}

#[doc(hidden)]
/// A trait for any object that can be Converted to a `HeaderValue`
pub trait IntoHeaderValue: Sized {
    /// The type returned in the event of a conversion error.
    type Error: Into<HttpError>;

    /// Try to convert value to a Header value.
    fn try_into(self) -> Result<HeaderValue, Self::Error>;
}

impl IntoHeaderValue for HeaderValue {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        Ok(self)
    }
}

impl<'a> IntoHeaderValue for &'a str {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        self.parse()
    }
}

impl<'a> IntoHeaderValue for &'a [u8] {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_bytes(self)
    }
}

impl IntoHeaderValue for Bytes {
    type Error = InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_shared(self)
    }
}

impl IntoHeaderValue for Vec<u8> {
    type Error = InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_shared(Bytes::from(self))
    }
}

impl IntoHeaderValue for String {
    type Error = InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_shared(Bytes::from(self))
    }
}

impl IntoHeaderValue for Mime {
    type Error = InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_shared(Bytes::from(format!("{}", self)))
    }
}

/// Represents supported types of content encodings
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation
    Auto,
    /// A format using the Brotli algorithm
    #[cfg(feature = "brotli")]
    Br,
    /// A format using the zlib structure with deflate algorithm
    #[cfg(feature = "flate2")]
    Deflate,
    /// Gzip algorithm
    #[cfg(feature = "flate2")]
    Gzip,
    /// Indicates the identity function (i.e. no compression, nor modification)
    Identity,
}

impl ContentEncoding {
    #[inline]
    /// Is the content compressed?
    pub fn is_compression(&self) -> bool {
        match *self {
            ContentEncoding::Identity | ContentEncoding::Auto => false,
            _ => true,
        }
    }

    #[inline]
    /// Convert content encoding to string
    pub fn as_str(&self) -> &'static str {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => "br",
            #[cfg(feature = "flate2")]
            ContentEncoding::Gzip => "gzip",
            #[cfg(feature = "flate2")]
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }

    #[inline]
    /// default quality value
    pub fn quality(&self) -> f64 {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => 1.1,
            #[cfg(feature = "flate2")]
            ContentEncoding::Gzip => 1.0,
            #[cfg(feature = "flate2")]
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::Identity | ContentEncoding::Auto => 0.1,
        }
    }
}

// TODO: remove memory allocation
impl<'a> From<&'a str> for ContentEncoding {
    fn from(s: &'a str) -> ContentEncoding {
        match AsRef::<str>::as_ref(&s.trim().to_lowercase()) {
            #[cfg(feature = "brotli")]
            "br" => ContentEncoding::Br,
            #[cfg(feature = "flate2")]
            "gzip" => ContentEncoding::Gzip,
            #[cfg(feature = "flate2")]
            "deflate" => ContentEncoding::Deflate,
            _ => ContentEncoding::Identity,
        }
    }
}

#[doc(hidden)]
pub(crate) struct Writer {
    buf: BytesMut,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            buf: BytesMut::new(),
        }
    }
    fn take(&mut self) -> Bytes {
        self.buf.take().freeze()
    }
}

impl fmt::Write for Writer {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }

    #[inline]
    fn write_fmt(&mut self, args: fmt::Arguments) -> fmt::Result {
        fmt::write(self, args)
    }
}

#[inline]
#[doc(hidden)]
/// Reads a comma-delimited raw header into a Vec.
pub fn from_comma_delimited<T: FromStr>(
    all: GetAll<HeaderValue>,
) -> Result<Vec<T>, ParseError> {
    let mut result = Vec::new();
    for h in all {
        let s = h.to_str().map_err(|_| ParseError::Header)?;
        result.extend(
            s.split(',')
                .filter_map(|x| match x.trim() {
                    "" => None,
                    y => Some(y),
                })
                .filter_map(|x| x.trim().parse().ok()),
        )
    }
    Ok(result)
}

#[inline]
#[doc(hidden)]
/// Reads a single string when parsing a header.
pub fn from_one_raw_str<T: FromStr>(val: Option<&HeaderValue>) -> Result<T, ParseError> {
    if let Some(line) = val {
        let line = line.to_str().map_err(|_| ParseError::Header)?;
        if !line.is_empty() {
            return T::from_str(line).or(Err(ParseError::Header));
        }
    }
    Err(ParseError::Header)
}

#[inline]
#[doc(hidden)]
/// Format an array into a comma-delimited string.
pub fn fmt_comma_delimited<T>(f: &mut fmt::Formatter, parts: &[T]) -> fmt::Result
where
    T: fmt::Display,
{
    let mut iter = parts.iter();
    if let Some(part) = iter.next() {
        fmt::Display::fmt(part, f)?;
    }
    for part in iter {
        f.write_str(", ")?;
        fmt::Display::fmt(part, f)?;
    }
    Ok(())
}

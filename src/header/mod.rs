//! Various http headers
// This is mostly copy of [hyper](https://github.com/hyperium/hyper/tree/master/src/header)

use std::fmt;
use std::str::FromStr;

use bytes::{Bytes, BytesMut};
use http::{Error as HttpError};
use http::header::GetAll;
use mime::Mime;

pub use cookie::{Cookie, CookieBuilder};
pub use http_range::HttpRange;

#[doc(hidden)]
pub mod http {
    pub use http::header::*;
}

use error::ParseError;
use httpmessage::HttpMessage;
pub use httpresponse::ConnectionType;

mod common;
mod shared;
#[doc(hidden)]
pub use self::common::*;
#[doc(hidden)]
pub use self::shared::*;


#[doc(hidden)]
/// A trait for any object that will represent a header field and value.
pub trait Header where Self: IntoHeaderValue {

    /// Returns the name of the header field
    fn name() -> http::HeaderName;

    /// Parse a header
    fn parse<T: HttpMessage>(msg: &T) -> Result<Self, ParseError>;
}

#[doc(hidden)]
/// A trait for any object that can be Converted to a `HeaderValue`
pub trait IntoHeaderValue: Sized {
    /// The type returned in the event of a conversion error.
    type Error: Into<HttpError>;

    /// Cast from PyObject to a concrete Python object type.
    fn try_into(self) -> Result<http::HeaderValue, Self::Error>;
}

impl IntoHeaderValue for http::HeaderValue {
    type Error = http::InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        Ok(self)
    }
}

impl<'a> IntoHeaderValue for &'a str {
    type Error = http::InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        self.parse()
    }
}

impl<'a> IntoHeaderValue for &'a [u8] {
    type Error = http::InvalidHeaderValue;

    #[inline]
    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        http::HeaderValue::from_bytes(self)
    }
}

impl IntoHeaderValue for Bytes {
    type Error = http::InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        http::HeaderValue::from_shared(self)
    }
}

impl IntoHeaderValue for Vec<u8> {
    type Error = http::InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        http::HeaderValue::from_shared(Bytes::from(self))
    }
}

impl IntoHeaderValue for String {
    type Error = http::InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        http::HeaderValue::from_shared(Bytes::from(self))
    }
}

impl IntoHeaderValue for Mime {
    type Error = http::InvalidHeaderValueBytes;

    #[inline]
    fn try_into(self) -> Result<http::HeaderValue, Self::Error> {
        http::HeaderValue::from_shared(Bytes::from(format!("{}", self)))
    }
}

/// Represents supported types of content encodings
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation
    Auto,
    /// A format using the Brotli algorithm
    #[cfg(feature="brotli")]
    Br,
    /// A format using the zlib structure with deflate algorithm
    Deflate,
    /// Gzip algorithm
    Gzip,
    /// Indicates the identity function (i.e. no compression, nor modification)
    Identity,
}

impl ContentEncoding {

    #[inline]
    pub fn is_compression(&self) -> bool {
        match *self {
            ContentEncoding::Identity | ContentEncoding::Auto => false,
            _ => true
        }
    }

    #[inline]
    pub fn as_str(&self) -> &'static str {
        match *self {
            #[cfg(feature="brotli")]
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }

    #[inline]
    /// default quality value
    pub fn quality(&self) -> f64 {
        match *self {
            #[cfg(feature="brotli")]
            ContentEncoding::Br => 1.1,
            ContentEncoding::Gzip => 1.0,
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::Identity | ContentEncoding::Auto => 0.1,
        }
    }
}

// TODO: remove memory allocation
impl<'a> From<&'a str> for ContentEncoding {
    fn from(s: &'a str) -> ContentEncoding {
        match s.trim().to_lowercase().as_ref() {
            #[cfg(feature="brotli")]
            "br" => ContentEncoding::Br,
            "gzip" => ContentEncoding::Gzip,
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
        Writer{buf: BytesMut::new()}
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
pub fn from_comma_delimited<T: FromStr>(all: GetAll<http::HeaderValue>)
                                        -> Result<Vec<T>, ParseError>
{
    let mut result = Vec::new();
    for h in all {
        let s = h.to_str().map_err(|_| ParseError::Header)?;
        result.extend(s.split(',')
                      .filter_map(|x| match x.trim() {
                          "" => None,
                          y => Some(y)
                      })
                      .filter_map(|x| x.trim().parse().ok()))
    }
    Ok(result)
}

#[inline]
#[doc(hidden)]
/// Reads a single string when parsing a header.
pub fn from_one_raw_str<T: FromStr>(val: Option<&http::HeaderValue>)
                                    -> Result<T, ParseError>
{
    if let Some(line) = val {
        let line = line.to_str().map_err(|_| ParseError::Header)?;
        if !line.is_empty() {
            return T::from_str(line).or(Err(ParseError::Header))
        }
    }
    Err(ParseError::Header)
}

#[inline]
#[doc(hidden)]
/// Format an array into a comma-delimited string.
pub fn fmt_comma_delimited<T>(f: &mut fmt::Formatter, parts: &[T]) -> fmt::Result
    where T: fmt::Display
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

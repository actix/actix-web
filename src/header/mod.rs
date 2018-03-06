//! Various http headers
// A lot of code is inspired by hyper

use bytes::Bytes;
use http::{Error as HttpError};
use http::header::{InvalidHeaderValue, InvalidHeaderValueBytes};

pub use cookie::{Cookie, CookieBuilder};
pub use http_range::HttpRange;
pub use http::header::{HeaderName, HeaderValue};

use error::ParseError;
use httpmessage::HttpMessage;
pub use httpresponse::ConnectionType;

mod common;
mod httpdate;
pub use self::common::*;
pub use self::httpdate::HttpDate;


#[doc(hidden)]
/// A trait for any object that will represent a header field and value.
pub trait Header where Self: IntoHeaderValue {

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

    /// Cast from PyObject to a concrete Python object type.
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

/// Represents supported types of content encodings
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation
    Auto,
    /// A format using the Brotli algorithm
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
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }
    /// default quality value
    pub fn quality(&self) -> f64 {
        match *self {
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
            "br" => ContentEncoding::Br,
            "gzip" => ContentEncoding::Gzip,
            "deflate" => ContentEncoding::Deflate,
            "identity" => ContentEncoding::Identity,
            _ => ContentEncoding::Auto,
        }
    }
}

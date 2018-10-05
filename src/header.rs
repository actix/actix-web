//! Various http headers

use bytes::Bytes;
use mime::Mime;
use modhttp::Error as HttpError;

pub use modhttp::header::*;

use error::ParseError;
use httpmessage::HttpMessage;

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
    /// Is the content compressed?
    pub fn is_compression(self) -> bool {
        match self {
            ContentEncoding::Identity | ContentEncoding::Auto => false,
            _ => true,
        }
    }

    #[inline]
    /// Convert content encoding to string
    pub fn as_str(self) -> &'static str {
        match self {
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }

    #[inline]
    /// default quality value
    pub fn quality(self) -> f64 {
        match self {
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
        match AsRef::<str>::as_ref(&s.trim().to_lowercase()) {
            "br" => ContentEncoding::Br,
            "gzip" => ContentEncoding::Gzip,
            "deflate" => ContentEncoding::Deflate,
            _ => ContentEncoding::Identity,
        }
    }
}

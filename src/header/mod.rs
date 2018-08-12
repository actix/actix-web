//! Various http headers
// This is mostly copy of [hyper](https://github.com/hyperium/hyper/tree/master/src/header)

use std::fmt;
use std::str::FromStr;

use bytes::{Bytes, BytesMut};
use mime::Mime;
use modhttp::header::GetAll;
use modhttp::Error as HttpError;
use percent_encoding;

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
    pub fn quality(self) -> f64 {
        match self {
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

// From hyper v0.11.27 src/header/parsing.rs

/// The value part of an extended parameter consisting of three parts:
/// the REQUIRED character set name (`charset`), the OPTIONAL language information (`language_tag`),
/// and a character sequence representing the actual value (`value`), separated by single quote
/// characters. It is defined in [RFC 5987](https://tools.ietf.org/html/rfc5987#section-3.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ExtendedValue {
    /// The character set that is used to encode the `value` to a string.
    pub charset: Charset,
    /// The human language details of the `value`, if available.
    pub language_tag: Option<LanguageTag>,
    /// The parameter value, as expressed in octets.
    pub value: Vec<u8>,
}

/// Parses extended header parameter values (`ext-value`), as defined in
/// [RFC 5987](https://tools.ietf.org/html/rfc5987#section-3.2).
///
/// Extended values are denoted by parameter names that end with `*`.
///
/// ## ABNF
///
/// ```text
/// ext-value     = charset  "'" [ language ] "'" value-chars
///               ; like RFC 2231's <extended-initial-value>
///               ; (see [RFC2231], Section 7)
///
/// charset       = "UTF-8" / "ISO-8859-1" / mime-charset
///
/// mime-charset  = 1*mime-charsetc
/// mime-charsetc = ALPHA / DIGIT
///               / "!" / "#" / "$" / "%" / "&"
///               / "+" / "-" / "^" / "_" / "`"
///               / "{" / "}" / "~"
///               ; as <mime-charset> in Section 2.3 of [RFC2978]
///               ; except that the single quote is not included
///               ; SHOULD be registered in the IANA charset registry
///
/// language      = <Language-Tag, defined in [RFC5646], Section 2.1>
///
/// value-chars   = *( pct-encoded / attr-char )
///
/// pct-encoded   = "%" HEXDIG HEXDIG
///               ; see [RFC3986], Section 2.1
///
/// attr-char     = ALPHA / DIGIT
///               / "!" / "#" / "$" / "&" / "+" / "-" / "."
///               / "^" / "_" / "`" / "|" / "~"
///               ; token except ( "*" / "'" / "%" )
/// ```
pub fn parse_extended_value(val: &str) -> Result<ExtendedValue, ::error::ParseError> {
    // Break into three pieces separated by the single-quote character
    let mut parts = val.splitn(3, '\'');

    // Interpret the first piece as a Charset
    let charset: Charset = match parts.next() {
        None => return Err(::error::ParseError::Header),
        Some(n) => FromStr::from_str(n).map_err(|_| ::error::ParseError::Header)?,
    };

    // Interpret the second piece as a language tag
    let language_tag: Option<LanguageTag> = match parts.next() {
        None => return Err(::error::ParseError::Header),
        Some("") => None,
        Some(s) => match s.parse() {
            Ok(lt) => Some(lt),
            Err(_) => return Err(::error::ParseError::Header),
        },
    };

    // Interpret the third piece as a sequence of value characters
    let value: Vec<u8> = match parts.next() {
        None => return Err(::error::ParseError::Header),
        Some(v) => percent_encoding::percent_decode(v.as_bytes()).collect(),
    };

    Ok(ExtendedValue {
        value,
        charset,
        language_tag,
    })
}

impl fmt::Display for ExtendedValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let encoded_value = percent_encoding::percent_encode(
            &self.value[..],
            self::percent_encoding_http::HTTP_VALUE,
        );
        if let Some(ref lang) = self.language_tag {
            write!(f, "{}'{}'{}", self.charset, lang, encoded_value)
        } else {
            write!(f, "{}''{}", self.charset, encoded_value)
        }
    }
}

/// Percent encode a sequence of bytes with a character set defined in
/// [https://tools.ietf.org/html/rfc5987#section-3.2][url]
///
/// [url]: https://tools.ietf.org/html/rfc5987#section-3.2
pub fn http_percent_encode(f: &mut fmt::Formatter, bytes: &[u8]) -> fmt::Result {
    let encoded =
        percent_encoding::percent_encode(bytes, self::percent_encoding_http::HTTP_VALUE);
    fmt::Display::fmt(&encoded, f)
}
mod percent_encoding_http {
    use percent_encoding;

    // internal module because macro is hard-coded to make a public item
    // but we don't want to public export this item
    define_encode_set! {
        // This encode set is used for HTTP header values and is defined at
        // https://tools.ietf.org/html/rfc5987#section-3.2
        pub HTTP_VALUE = [percent_encoding::SIMPLE_ENCODE_SET] | {
            ' ', '"', '%', '\'', '(', ')', '*', ',', '/', ':', ';', '<', '-', '>', '?',
            '[', '\\', ']', '{', '}'
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_extended_value, ExtendedValue};
    use header::shared::Charset;
    use language_tags::LanguageTag;

    #[test]
    fn test_parse_extended_value_with_encoding_and_language_tag() {
        let expected_language_tag = "en".parse::<LanguageTag>().unwrap();
        // RFC 5987, Section 3.2.2
        // Extended notation, using the Unicode character U+00A3 (POUND SIGN)
        let result = parse_extended_value("iso-8859-1'en'%A3%20rates");
        assert!(result.is_ok());
        let extended_value = result.unwrap();
        assert_eq!(Charset::Iso_8859_1, extended_value.charset);
        assert!(extended_value.language_tag.is_some());
        assert_eq!(expected_language_tag, extended_value.language_tag.unwrap());
        assert_eq!(
            vec![163, b' ', b'r', b'a', b't', b'e', b's'],
            extended_value.value
        );
    }

    #[test]
    fn test_parse_extended_value_with_encoding() {
        // RFC 5987, Section 3.2.2
        // Extended notation, using the Unicode characters U+00A3 (POUND SIGN)
        // and U+20AC (EURO SIGN)
        let result = parse_extended_value("UTF-8''%c2%a3%20and%20%e2%82%ac%20rates");
        assert!(result.is_ok());
        let extended_value = result.unwrap();
        assert_eq!(Charset::Ext("UTF-8".to_string()), extended_value.charset);
        assert!(extended_value.language_tag.is_none());
        assert_eq!(
            vec![
                194, 163, b' ', b'a', b'n', b'd', b' ', 226, 130, 172, b' ', b'r', b'a',
                b't', b'e', b's',
            ],
            extended_value.value
        );
    }

    #[test]
    fn test_parse_extended_value_missing_language_tag_and_encoding() {
        // From: https://greenbytes.de/tech/tc2231/#attwithfn2231quot2
        let result = parse_extended_value("foo%20bar.html");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_extended_value_partially_formatted() {
        let result = parse_extended_value("UTF-8'missing third part");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_extended_value_partially_formatted_blank() {
        let result = parse_extended_value("blank second part'");
        assert!(result.is_err());
    }

    #[test]
    fn test_fmt_extended_value_with_encoding_and_language_tag() {
        let extended_value = ExtendedValue {
            charset: Charset::Iso_8859_1,
            language_tag: Some("en".parse().expect("Could not parse language tag")),
            value: vec![163, b' ', b'r', b'a', b't', b'e', b's'],
        };
        assert_eq!("ISO-8859-1'en'%A3%20rates", format!("{}", extended_value));
    }

    #[test]
    fn test_fmt_extended_value_with_encoding() {
        let extended_value = ExtendedValue {
            charset: Charset::Ext("UTF-8".to_string()),
            language_tag: None,
            value: vec![
                194, 163, b' ', b'a', b'n', b'd', b' ', 226, 130, 172, b' ', b'r', b'a',
                b't', b'e', b's',
            ],
        };
        assert_eq!(
            "UTF-8''%C2%A3%20and%20%E2%82%AC%20rates",
            format!("{}", extended_value)
        );
    }
}

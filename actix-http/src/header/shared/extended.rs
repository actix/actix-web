//! Originally taken from `hyper::header::parsing`.

use std::{fmt, str::FromStr};

use language_tags::LanguageTag;

use crate::header::{Charset, HTTP_VALUE};

/// The value part of an extended parameter consisting of three parts:
/// - The REQUIRED character set name (`charset`).
/// - The OPTIONAL language information (`language_tag`).
/// - A character sequence representing the actual value (`value`), separated by single quotes.
///
/// It is defined in [RFC 5987 §3.2](https://datatracker.ietf.org/doc/html/rfc5987#section-3.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtendedValue {
    /// The character set that is used to encode the `value` to a string.
    pub charset: Charset,

    /// The human language details of the `value`, if available.
    pub language_tag: Option<LanguageTag>,

    /// The parameter value, as expressed in octets.
    pub value: Vec<u8>,
}

/// Parses extended header parameter values (`ext-value`), as defined
/// in [RFC 5987 §3.2](https://datatracker.ietf.org/doc/html/rfc5987#section-3.2).
///
/// Extended values are denoted by parameter names that end with `*`.
///
/// ## ABNF
///
/// ```plain
/// ext-value     = charset  "'" [ language ] "'" value-chars
///               ; like RFC 2231's <extended-initial-value>
///               ; (see [RFC 2231 §7])
///
/// charset       = "UTF-8" / "ISO-8859-1" / mime-charset
///
/// mime-charset  = 1*mime-charsetc
/// mime-charsetc = ALPHA / DIGIT
///               / "!" / "#" / "$" / "%" / "&"
///               / "+" / "-" / "^" / "_" / "`"
///               / "{" / "}" / "~"
///               ; as <mime-charset> in [RFC 2978 §2.3]
///               ; except that the single quote is not included
///               ; SHOULD be registered in the IANA charset registry
///
/// language      = <Language-Tag, defined in [RFC 5646 §2.1]>
///
/// value-chars   = *( pct-encoded / attr-char )
///
/// pct-encoded   = "%" HEXDIG HEXDIG
///               ; see [RFC 3986 §2.1]
///
/// attr-char     = ALPHA / DIGIT
///               / "!" / "#" / "$" / "&" / "+" / "-" / "."
///               / "^" / "_" / "`" / "|" / "~"
///               ; token except ( "*" / "'" / "%" )
/// ```
///
/// [RFC 2231 §7]: https://datatracker.ietf.org/doc/html/rfc2231#section-7
/// [RFC 2978 §2.3]: https://datatracker.ietf.org/doc/html/rfc2978#section-2.3
/// [RFC 3986 §2.1]: https://datatracker.ietf.org/doc/html/rfc5646#section-2.1
pub fn parse_extended_value(val: &str) -> Result<ExtendedValue, crate::error::ParseError> {
    // Break into three pieces separated by the single-quote character
    let mut parts = val.splitn(3, '\'');

    // Interpret the first piece as a Charset
    let charset: Charset = match parts.next() {
        None => return Err(crate::error::ParseError::Header),
        Some(n) => FromStr::from_str(n).map_err(|_| crate::error::ParseError::Header)?,
    };

    // Interpret the second piece as a language tag
    let language_tag: Option<LanguageTag> = match parts.next() {
        None => return Err(crate::error::ParseError::Header),
        Some("") => None,
        Some(s) => match s.parse() {
            Ok(lt) => Some(lt),
            Err(_) => return Err(crate::error::ParseError::Header),
        },
    };

    // Interpret the third piece as a sequence of value characters
    let value: Vec<u8> = match parts.next() {
        None => return Err(crate::error::ParseError::Header),
        Some(v) => percent_encoding::percent_decode(v.as_bytes()).collect(),
    };

    Ok(ExtendedValue {
        charset,
        language_tag,
        value,
    })
}

impl fmt::Display for ExtendedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let encoded_value = percent_encoding::percent_encode(&self.value[..], HTTP_VALUE);
        if let Some(ref lang) = self.language_tag {
            write!(f, "{}'{}'{}", self.charset, lang, encoded_value)
        } else {
            write!(f, "{}''{}", self.charset, encoded_value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                194, 163, b' ', b'a', b'n', b'd', b' ', 226, 130, 172, b' ', b'r', b'a', b't',
                b'e', b's',
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
                194, 163, b' ', b'a', b'n', b'd', b' ', 226, 130, 172, b' ', b'r', b'a', b't',
                b'e', b's',
            ],
        };
        assert_eq!(
            "UTF-8''%C2%A3%20and%20%E2%82%AC%20rates",
            format!("{}", extended_value)
        );
    }
}

use std::{
    fmt::{self, Display, Write},
    str::FromStr,
};

use super::{HeaderValue, InvalidHeaderValue, TryIntoHeaderValue, Writer, CONTENT_RANGE};
use crate::error::ParseError;

crate::http::header::common_header! {
    /// `Content-Range` header, defined
    /// in [RFC 7233 §4.2](https://datatracker.ietf.org/doc/html/rfc7233#section-4.2)
    (ContentRange, CONTENT_RANGE) => [ContentRangeSpec]

    test_parse_and_format {
        crate::http::header::common_header_test!(test_bytes,
            [b"bytes 0-499/500"],
            Some(ContentRange(ContentRangeSpec::Bytes {
                range: Some((0, 499)),
                instance_length: Some(500)
            })));

        crate::http::header::common_header_test!(test_bytes_unknown_len,
            [b"bytes 0-499/*"],
            Some(ContentRange(ContentRangeSpec::Bytes {
                range: Some((0, 499)),
                instance_length: None
            })));

        crate::http::header::common_header_test!(test_bytes_unknown_range,
            [b"bytes */500"],
            Some(ContentRange(ContentRangeSpec::Bytes {
                range: None,
                instance_length: Some(500)
            })));

        crate::http::header::common_header_test!(test_unregistered,
            [b"seconds 1-2"],
            Some(ContentRange(ContentRangeSpec::Unregistered {
                unit: "seconds".to_owned(),
                resp: "1-2".to_owned()
            })));

        crate::http::header::common_header_test!(test_no_len,
            [b"bytes 0-499"],
            None::<ContentRange>);

        crate::http::header::common_header_test!(test_only_unit,
            [b"bytes"],
            None::<ContentRange>);

        crate::http::header::common_header_test!(test_end_less_than_start,
            [b"bytes 499-0/500"],
            None::<ContentRange>);

        crate::http::header::common_header_test!(test_blank,
            [b""],
            None::<ContentRange>);

        crate::http::header::common_header_test!(test_bytes_many_spaces,
            [b"bytes 1-2/500 3"],
            None::<ContentRange>);

        crate::http::header::common_header_test!(test_bytes_many_slashes,
            [b"bytes 1-2/500/600"],
            None::<ContentRange>);

        crate::http::header::common_header_test!(test_bytes_many_dashes,
            [b"bytes 1-2-3/500"],
            None::<ContentRange>);
    }
}

/// Content-Range header, defined
/// in [RFC 7233 §4.2](https://datatracker.ietf.org/doc/html/rfc7233#section-4.2)
///
/// # ABNF
/// ```plain
/// Content-Range       = byte-content-range
///                     / other-content-range
///
/// byte-content-range  = bytes-unit SP
///                       ( byte-range-resp / unsatisfied-range )
///
/// byte-range-resp     = byte-range "/" ( complete-length / "*" )
/// byte-range          = first-byte-pos "-" last-byte-pos
/// unsatisfied-range   = "*/" complete-length
///
/// complete-length     = 1*DIGIT
///
/// other-content-range = other-range-unit SP other-range-resp
/// other-range-resp    = *CHAR
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentRangeSpec {
    /// Byte range
    Bytes {
        /// First and last bytes of the range, omitted if request could not be
        /// satisfied
        range: Option<(u64, u64)>,

        /// Total length of the instance, can be omitted if unknown
        instance_length: Option<u64>,
    },

    /// Custom range, with unit not registered at IANA
    Unregistered {
        /// other-range-unit
        unit: String,

        /// other-range-resp
        resp: String,
    },
}

impl FromStr for ContentRangeSpec {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        let res = match s.split_once(' ') {
            Some(("bytes", resp)) => {
                let (range, instance_length) = resp.split_once('/').ok_or(ParseError::Header)?;

                let instance_length = if instance_length == "*" {
                    None
                } else {
                    Some(instance_length.parse().map_err(|_| ParseError::Header)?)
                };

                let range = if range == "*" {
                    None
                } else {
                    let (first_byte, last_byte) =
                        range.split_once('-').ok_or(ParseError::Header)?;
                    let first_byte = first_byte.parse().map_err(|_| ParseError::Header)?;
                    let last_byte = last_byte.parse().map_err(|_| ParseError::Header)?;
                    if last_byte < first_byte {
                        return Err(ParseError::Header);
                    }
                    Some((first_byte, last_byte))
                };

                ContentRangeSpec::Bytes {
                    range,
                    instance_length,
                }
            }
            Some((unit, resp)) => ContentRangeSpec::Unregistered {
                unit: unit.to_owned(),
                resp: resp.to_owned(),
            },
            _ => return Err(ParseError::Header),
        };
        Ok(res)
    }
}

impl Display for ContentRangeSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ContentRangeSpec::Bytes {
                range,
                instance_length,
            } => {
                f.write_str("bytes ")?;
                match range {
                    Some((first_byte, last_byte)) => {
                        write!(f, "{}-{}", first_byte, last_byte)?;
                    }
                    None => {
                        f.write_str("*")?;
                    }
                };
                f.write_str("/")?;
                if let Some(v) = instance_length {
                    write!(f, "{}", v)
                } else {
                    f.write_str("*")
                }
            }
            ContentRangeSpec::Unregistered { ref unit, ref resp } => {
                f.write_str(unit)?;
                f.write_str(" ")?;
                f.write_str(resp)
            }
        }
    }
}

impl TryIntoHeaderValue for ContentRangeSpec {
    type Error = InvalidHeaderValue;

    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        let mut writer = Writer::new();
        let _ = write!(&mut writer, "{}", self);
        HeaderValue::from_maybe_shared(writer.take())
    }
}

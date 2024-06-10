//! Header parsing utilities.

use std::{fmt, str::FromStr};

use super::HeaderValue;
use crate::{error::ParseError, header::HTTP_VALUE};

/// Reads a comma-delimited raw header into a Vec.
#[inline]
pub fn from_comma_delimited<'a, I, T>(all: I) -> Result<Vec<T>, ParseError>
where
    I: Iterator<Item = &'a HeaderValue> + 'a,
    T: FromStr,
{
    let size_guess = all.size_hint().1.unwrap_or(2);
    let mut result = Vec::with_capacity(size_guess);

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

/// Reads a single string when parsing a header.
#[inline]
pub fn from_one_raw_str<T: FromStr>(val: Option<&HeaderValue>) -> Result<T, ParseError> {
    if let Some(line) = val {
        let line = line.to_str().map_err(|_| ParseError::Header)?;

        if !line.is_empty() {
            return T::from_str(line).or(Err(ParseError::Header));
        }
    }

    Err(ParseError::Header)
}

/// Format an array into a comma-delimited string.
#[inline]
pub fn fmt_comma_delimited<T>(f: &mut fmt::Formatter<'_>, parts: &[T]) -> fmt::Result
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

/// Percent encode a sequence of bytes with a character set defined in [RFC 5987 §3.2].
///
/// [RFC 5987 §3.2]: https://datatracker.ietf.org/doc/html/rfc5987#section-3.2
#[inline]
pub fn http_percent_encode(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    let encoded = percent_encoding::percent_encode(bytes, HTTP_VALUE);
    fmt::Display::fmt(&encoded, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comma_delimited_parsing() {
        let headers = [];
        let res: Vec<usize> = from_comma_delimited(headers.iter()).unwrap();
        assert_eq!(res, vec![0; 0]);

        let headers = [
            HeaderValue::from_static("1, 2"),
            HeaderValue::from_static("3,4"),
        ];
        let res: Vec<usize> = from_comma_delimited(headers.iter()).unwrap();
        assert_eq!(res, vec![1, 2, 3, 4]);

        let headers = [
            HeaderValue::from_static(""),
            HeaderValue::from_static(","),
            HeaderValue::from_static("  "),
            HeaderValue::from_static("1    ,"),
            HeaderValue::from_static(""),
        ];
        let res: Vec<usize> = from_comma_delimited(headers.iter()).unwrap();
        assert_eq!(res, vec![1]);
    }
}

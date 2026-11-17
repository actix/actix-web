//! Header parsing utilities.

use std::{fmt, str::FromStr};

use super::HeaderValue;
use crate::{error::ParseError, header::HTTP_VALUE};

/// Reads a comma-delimited raw header into a Vec.
///
/// A comma inside a quoted string does not delimit list elements. See [RFC 9110 §5.6.1] and
/// [RFC 9110 §5.6.4].
///
/// [RFC 9110 §5.6.1]: https://www.rfc-editor.org/rfc/rfc9110#section-5.6.1
/// [RFC 9110 §5.6.4]: https://www.rfc-editor.org/rfc/rfc9110#section-5.6.4
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
            split_list_elements(s)
                .filter_map(|x| match x.trim() {
                    "" => None,
                    y => Some(y),
                })
                .filter_map(|x| x.trim().parse().ok()),
        )
    }

    Ok(result)
}

/// Splits a field value on the commas that are outside quoted strings.
///
/// A backslash inside a quoted string escapes the next character (a quoted-pair), so an
/// escaped double quote does not end the quoted string.
fn split_list_elements(value: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(value);

    std::iter::from_fn(move || {
        let value = rest?;
        let mut in_quoted_string = false;
        let mut escaped = false;

        for (idx, byte) in value.bytes().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }

            match byte {
                b'\\' if in_quoted_string => escaped = true,
                b'"' => in_quoted_string = !in_quoted_string,
                b',' if !in_quoted_string => {
                    rest = Some(&value[idx + 1..]);
                    return Some(&value[..idx]);
                }
                _ => {}
            }
        }

        rest = None;
        Some(value)
    })
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

    #[test]
    fn comma_delimited_parsing_keeps_commas_in_quoted_strings() {
        let headers = [
            HeaderValue::from_static(r#""a,b", "c""#),
            HeaderValue::from_static(r#"private="x, y", no-store"#),
        ];
        let res: Vec<String> = from_comma_delimited(headers.iter()).unwrap();
        assert_eq!(res, [r#""a,b""#, r#""c""#, r#"private="x, y""#, "no-store"]);
    }

    #[test]
    fn comma_delimited_parsing_skips_quoted_pairs() {
        let headers = [HeaderValue::from_static(r#"a="x\", y", b"#)];
        let res: Vec<String> = from_comma_delimited(headers.iter()).unwrap();
        assert_eq!(res, [r#"a="x\", y""#, "b"]);
    }
}

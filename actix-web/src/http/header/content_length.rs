use std::{convert::Infallible, str};

use derive_more::{Deref, DerefMut};

use crate::{
    error::ParseError,
    http::header::{
        from_one_raw_str, Header, HeaderName, HeaderValue, TryIntoHeaderValue, CONTENT_LENGTH,
    },
    HttpMessage,
};

/// `Content-Length` header, defined in [RFC 9110 §8.6].
///
/// The Content-Length
///
/// # ABNF
///
/// ```plain
/// Content-Length = 1*DIGIT
/// ```
///
/// # Example Values
///
/// - `0`
/// - `3495`
///
/// # Examples
///
/// ```
/// use actix_web::{http::header::ContentLength, HttpResponse};
///
/// let res_empty = HttpResponse::Ok()
///     .insert_header(ContentLength(0));
///
/// let res_fake_cl = HttpResponse::Ok()
///     .insert_header(ContentLength(3_495));
/// ```
///
/// [RFC 9110 §8.6]: https://www.rfc-editor.org/rfc/rfc9110#name-content-length
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deref, DerefMut)]
pub struct ContentLength(pub usize);

impl ContentLength {
    /// Returns Content-Length value.
    pub fn into_inner(&self) -> usize {
        self.0
    }
}

impl str::FromStr for ContentLength {
    type Err = <usize as str::FromStr>::Err;

    #[inline]
    fn from_str(val: &str) -> Result<Self, Self::Err> {
        let val = val.trim();

        // decoder prevents this case
        debug_assert!(!val.starts_with('+'));

        val.parse().map(Self)
    }
}

impl TryIntoHeaderValue for ContentLength {
    type Error = Infallible;

    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        Ok(HeaderValue::from(self.0))
    }
}

impl Header for ContentLength {
    fn name() -> HeaderName {
        CONTENT_LENGTH
    }

    fn parse<M: HttpMessage>(msg: &M) -> Result<Self, ParseError> {
        let val = from_one_raw_str(msg.headers().get(Self::name()))?;

        // decoder prevents multiple CL headers
        debug_assert_eq!(msg.headers().get_all(Self::name()).count(), 1);

        Ok(val)
    }
}

impl From<ContentLength> for usize {
    fn from(ContentLength(len): ContentLength) -> Self {
        len
    }
}

impl From<usize> for ContentLength {
    fn from(len: usize) -> Self {
        ContentLength(len)
    }
}

impl PartialEq<usize> for ContentLength {
    fn eq(&self, other: &usize) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ContentLength> for usize {
    fn eq(&self, other: &ContentLength) -> bool {
        *self == other.0
    }
}

impl PartialOrd<usize> for ContentLength {
    fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

impl PartialOrd<ContentLength> for usize {
    fn partial_cmp(&self, other: &ContentLength) -> Option<std::cmp::Ordering> {
        self.partial_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;
    use crate::{test::TestRequest, HttpRequest};

    fn req_from_raw_headers<H: Header, I: IntoIterator<Item = V>, V: AsRef<[u8]>>(
        header_lines: I,
    ) -> HttpRequest {
        header_lines
            .into_iter()
            .fold(TestRequest::default(), |req, item| {
                req.append_header((H::name(), item.as_ref().to_vec()))
            })
            .to_http_request()
    }

    #[track_caller]
    pub(crate) fn assert_parse_fail<
        H: Header + fmt::Debug,
        I: IntoIterator<Item = V>,
        V: AsRef<[u8]>,
    >(
        headers: I,
    ) {
        let req = req_from_raw_headers::<H, _, _>(headers);
        H::parse(&req).unwrap_err();
    }

    #[track_caller]
    pub(crate) fn assert_parse_eq<
        H: Header + fmt::Debug + PartialEq,
        I: IntoIterator<Item = V>,
        V: AsRef<[u8]>,
    >(
        headers: I,
        expect: H,
    ) {
        let req = req_from_raw_headers::<H, _, _>(headers);
        assert_eq!(H::parse(&req).unwrap(), expect);
    }

    #[test]
    fn missing_header() {
        assert_parse_fail::<ContentLength, _, _>([""; 0]);
        assert_parse_fail::<ContentLength, _, _>([""]);
    }

    #[test]
    fn bad_header() {
        assert_parse_fail::<ContentLength, _, _>(["-123"]);
        assert_parse_fail::<ContentLength, _, _>(["123_456"]);
        assert_parse_fail::<ContentLength, _, _>(["123.456"]);

        // too large for u64 (2^64, 2^64 + 1)
        assert_parse_fail::<ContentLength, _, _>(["18446744073709551616"]);
        assert_parse_fail::<ContentLength, _, _>(["18446744073709551617"]);

        // hex notation
        assert_parse_fail::<ContentLength, _, _>(["0x123"]);

        // multi-value
        assert_parse_fail::<ContentLength, _, _>(["0, 123"]);
    }

    #[test]
    #[should_panic]
    fn bad_header_plus() {
        // prevented by HTTP decoder anyway
        assert_parse_fail::<ContentLength, _, _>(["+123"]);
    }

    #[test]
    #[should_panic]
    fn bad_multiple_value() {
        // prevented by HTTP decoder anyway
        assert_parse_fail::<ContentLength, _, _>(["0", "123"]);
    }

    #[test]
    fn good_header() {
        assert_parse_eq::<ContentLength, _, _>(["0"], ContentLength(0));
        assert_parse_eq::<ContentLength, _, _>(["1"], ContentLength(1));
        assert_parse_eq::<ContentLength, _, _>(["123"], ContentLength(123));

        // value that looks like octal notation is not interpreted as such
        assert_parse_eq::<ContentLength, _, _>(["0123"], ContentLength(123));

        // whitespace variations
        assert_parse_eq::<ContentLength, _, _>([" 0"], ContentLength(0));
        assert_parse_eq::<ContentLength, _, _>(["0 "], ContentLength(0));
        assert_parse_eq::<ContentLength, _, _>([" 0 "], ContentLength(0));

        // large value (2^64 - 1)
        assert_parse_eq::<ContentLength, _, _>(
            ["18446744073709551615"],
            ContentLength(18_446_744_073_709_551_615),
        );
    }

    #[test]
    fn equality() {
        assert!(ContentLength(0) == ContentLength(0));
        assert!(ContentLength(0) == 0);
        assert!(0 != ContentLength(123));
    }

    #[test]
    fn ordering() {
        assert!(ContentLength(0) < ContentLength(123));
        assert!(ContentLength(0) < 123);
        assert!(0 < ContentLength(123));
    }
}

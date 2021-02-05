use std::convert::TryFrom;

use bytes::Bytes;
use http::{header::InvalidHeaderValue, Error as HttpError, HeaderValue};
use mime::Mime;

/// A trait for any object that can be Converted to a `HeaderValue`
pub trait IntoHeaderValue: Sized {
    /// The type returned in the event of a conversion error.
    type Error: Into<HttpError>;

    /// Try to convert value to a HeaderValue.
    fn try_into_value(self) -> Result<HeaderValue, Self::Error>;
}

impl IntoHeaderValue for HeaderValue {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        Ok(self)
    }
}

impl IntoHeaderValue for &HeaderValue {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        Ok(self.clone())
    }
}

impl IntoHeaderValue for &str {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        self.parse()
    }
}

impl IntoHeaderValue for &[u8] {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_bytes(self)
    }
}

impl IntoHeaderValue for Bytes {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_maybe_shared(self)
    }
}

impl IntoHeaderValue for Vec<u8> {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self)
    }
}

impl IntoHeaderValue for String {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self)
    }
}

impl IntoHeaderValue for usize {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl IntoHeaderValue for i64 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl IntoHeaderValue for u64 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl IntoHeaderValue for i32 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl IntoHeaderValue for u32 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl IntoHeaderValue for Mime {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_str(self.as_ref())
    }
}

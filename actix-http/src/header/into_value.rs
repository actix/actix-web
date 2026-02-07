//! [`TryIntoHeaderValue`] trait and implementations.

use bytes::Bytes;
use http::{header::InvalidHeaderValue, Error as HttpError, HeaderValue};
use mime::Mime;

/// An interface for types that can be converted into a [`HeaderValue`].
pub trait TryIntoHeaderValue: Sized {
    /// The type returned in the event of a conversion error.
    type Error: Into<HttpError>;

    /// Try to convert value to a HeaderValue.
    fn try_into_value(self) -> Result<HeaderValue, Self::Error>;
}

impl TryIntoHeaderValue for HeaderValue {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        Ok(self)
    }
}

impl TryIntoHeaderValue for &HeaderValue {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        Ok(self.clone())
    }
}

impl TryIntoHeaderValue for &str {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        self.parse()
    }
}

impl TryIntoHeaderValue for &[u8] {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_bytes(self)
    }
}

impl TryIntoHeaderValue for Bytes {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_maybe_shared(self)
    }
}

impl TryIntoHeaderValue for Vec<u8> {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self)
    }
}

impl TryIntoHeaderValue for String {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self)
    }
}

impl TryIntoHeaderValue for usize {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl TryIntoHeaderValue for i64 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl TryIntoHeaderValue for u64 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl TryIntoHeaderValue for i32 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl TryIntoHeaderValue for u32 {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::try_from(self.to_string())
    }
}

impl TryIntoHeaderValue for Mime {
    type Error = InvalidHeaderValue;

    #[inline]
    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_str(self.as_ref())
    }
}

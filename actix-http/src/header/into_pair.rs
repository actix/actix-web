//! [`TryIntoHeaderPair`] trait and implementations.

use super::{
    Header, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue, TryIntoHeaderValue,
};
use crate::error::HttpError;

/// An interface for types that can be converted into a [`HeaderName`] + [`HeaderValue`] pair for
/// insertion into a [`HeaderMap`].
///
/// [`HeaderMap`]: super::HeaderMap
pub trait TryIntoHeaderPair: Sized {
    type Error: Into<HttpError>;

    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error>;
}

#[derive(Debug)]
pub enum InvalidHeaderPart {
    Name(InvalidHeaderName),
    Value(InvalidHeaderValue),
}

impl From<InvalidHeaderPart> for HttpError {
    fn from(part_err: InvalidHeaderPart) -> Self {
        match part_err {
            InvalidHeaderPart::Name(err) => err.into(),
            InvalidHeaderPart::Value(err) => err.into(),
        }
    }
}

impl<V> TryIntoHeaderPair for (HeaderName, V)
where
    V: TryIntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name, value))
    }
}

impl<V> TryIntoHeaderPair for (&HeaderName, V)
where
    V: TryIntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name.clone(), value))
    }
}

impl<V> TryIntoHeaderPair for (&[u8], V)
where
    V: TryIntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(name).map_err(InvalidHeaderPart::Name)?;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name, value))
    }
}

impl<V> TryIntoHeaderPair for (&str, V)
where
    V: TryIntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(name).map_err(InvalidHeaderPart::Name)?;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name, value))
    }
}

impl<V> TryIntoHeaderPair for (String, V)
where
    V: TryIntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    #[inline]
    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        (name.as_str(), value).try_into_pair()
    }
}

impl<T: Header> TryIntoHeaderPair for T {
    type Error = <T as TryIntoHeaderValue>::Error;

    #[inline]
    fn try_into_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        Ok((T::name(), self.try_into_value()?))
    }
}

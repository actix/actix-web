use std::convert::TryFrom;

use http::{
    header::{HeaderName, InvalidHeaderName, InvalidHeaderValue},
    Error as HttpError, HeaderValue,
};

use super::{Header, IntoHeaderValue};

/// Transforms structures into header K/V pairs for inserting into `HeaderMap`s.
pub trait IntoHeaderPair: Sized {
    type Error: Into<HttpError>;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error>;
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

impl<V> IntoHeaderPair for (HeaderName, V)
where
    V: IntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name, value))
    }
}

impl<V> IntoHeaderPair for (&HeaderName, V)
where
    V: IntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name.clone(), value))
    }
}

impl<V> IntoHeaderPair for (&[u8], V)
where
    V: IntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(name).map_err(InvalidHeaderPart::Name)?;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name, value))
    }
}

impl<V> IntoHeaderPair for (&str, V)
where
    V: IntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(name).map_err(InvalidHeaderPart::Name)?;
        let value = value
            .try_into_value()
            .map_err(|err| InvalidHeaderPart::Value(err.into()))?;
        Ok((name, value))
    }
}

impl<V> IntoHeaderPair for (String, V)
where
    V: IntoHeaderValue,
    V::Error: Into<InvalidHeaderValue>,
{
    type Error = InvalidHeaderPart;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        (name.as_str(), value).try_into_header_pair()
    }
}

impl<T: Header> IntoHeaderPair for T {
    type Error = <T as IntoHeaderValue>::Error;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        Ok((T::name(), self.try_into_value()?))
    }
}

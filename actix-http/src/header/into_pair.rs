use std::convert::{Infallible, TryFrom};

use either::Either;
use http::{
    header::{HeaderName, InvalidHeaderName, InvalidHeaderValue},
    Error as HttpError, HeaderValue,
};

use super::{Header, IntoHeaderValue};

/// Transforms structures into header K/V pairs for inserting into `HeaderMap`s.
pub trait IntoHeaderPair: Sized {
    type Error;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error>;
}

impl IntoHeaderPair for (HeaderName, HeaderValue) {
    type Error = Infallible;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        Ok(self)
    }
}

impl IntoHeaderPair for (HeaderName, &str) {
    type Error = InvalidHeaderValue;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let value = HeaderValue::try_from(value)?;
        Ok((name, value))
    }
}

impl IntoHeaderPair for (&str, HeaderValue) {
    type Error = InvalidHeaderName;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(name)?;
        Ok((name, value))
    }
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

impl IntoHeaderPair for (&str, &str) {
    type Error = InvalidHeaderPart;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(name).map_err(InvalidHeaderPart::Name)?;
        let value = HeaderValue::try_from(value).map_err(InvalidHeaderPart::Value)?;
        Ok((name, value))
    }
}

impl IntoHeaderPair for (HeaderName, String) {
    type Error = InvalidHeaderValue;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let value = HeaderValue::try_from(&value)?;
        Ok((name, value))
    }
}

impl IntoHeaderPair for (String, HeaderValue) {
    type Error = InvalidHeaderName;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(&name)?;
        Ok((name, value))
    }
}

impl IntoHeaderPair for (String, String) {
    type Error = Either<InvalidHeaderName, InvalidHeaderValue>;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(&name).map_err(Either::Left)?;
        let value = HeaderValue::try_from(&value).map_err(Either::Right)?;
        Ok((name, value))
    }
}

impl<T> IntoHeaderPair for T
where
    T: Header,
{
    type Error = <T as IntoHeaderValue>::Error;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        Ok((T::name(), self.try_into_value()?))
    }
}

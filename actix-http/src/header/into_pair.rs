use std::convert::{Infallible, TryFrom};

use either::Either;
use http::{
    header::{HeaderName, InvalidHeaderName, InvalidHeaderValue},
    HeaderValue,
};

/// A trait for transforming things
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

impl IntoHeaderPair for (&str, &str) {
    type Error = Either<InvalidHeaderName, InvalidHeaderValue>;

    fn try_into_header_pair(self) -> Result<(HeaderName, HeaderValue), Self::Error> {
        let (name, value) = self;
        let name = HeaderName::try_from(name).map_err(Either::Left)?;
        let value = HeaderValue::try_from(value).map_err(Either::Right)?;
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

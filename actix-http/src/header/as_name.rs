//! Helper trait for types that can be effectively borrowed as a [HeaderValue].
//!
//! [HeaderValue]: crate::http::HeaderValue

use std::{borrow::Cow, str::FromStr};

use http::header::{HeaderName, InvalidHeaderName};

pub trait AsHeaderName: Sealed {}

pub trait Sealed {
    fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName>;
}

impl Sealed for HeaderName {
    fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        Ok(Cow::Borrowed(self))
    }
}
impl AsHeaderName for HeaderName {}

impl Sealed for &HeaderName {
    fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        Ok(Cow::Borrowed(*self))
    }
}
impl AsHeaderName for &HeaderName {}

impl Sealed for &str {
    fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        HeaderName::from_str(self).map(Cow::Owned)
    }
}
impl AsHeaderName for &str {}

impl Sealed for String {
    fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        HeaderName::from_str(self).map(Cow::Owned)
    }
}
impl AsHeaderName for String {}

impl Sealed for &String {
    fn try_as_name(&self) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        HeaderName::from_str(self).map(Cow::Owned)
    }
}
impl AsHeaderName for &String {}

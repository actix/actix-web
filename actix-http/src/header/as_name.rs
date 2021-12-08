//! Sealed [`AsHeaderName`] trait and implementations.

use std::{borrow::Cow, str::FromStr as _};

use http::header::{HeaderName, InvalidHeaderName};

/// Sealed trait implemented for types that can be effectively borrowed as a [`HeaderValue`].
///
/// [`HeaderValue`]: super::HeaderValue
pub trait AsHeaderName: Sealed {}

pub struct Seal;

pub trait Sealed {
    fn try_as_name(&self, seal: Seal) -> Result<Cow<'_, HeaderName>, InvalidHeaderName>;
}

impl Sealed for HeaderName {
    fn try_as_name(&self, _: Seal) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        Ok(Cow::Borrowed(self))
    }
}
impl AsHeaderName for HeaderName {}

impl Sealed for &HeaderName {
    fn try_as_name(&self, _: Seal) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        Ok(Cow::Borrowed(*self))
    }
}
impl AsHeaderName for &HeaderName {}

impl Sealed for &str {
    fn try_as_name(&self, _: Seal) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        HeaderName::from_str(self).map(Cow::Owned)
    }
}
impl AsHeaderName for &str {}

impl Sealed for String {
    fn try_as_name(&self, _: Seal) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        HeaderName::from_str(self).map(Cow::Owned)
    }
}
impl AsHeaderName for String {}

impl Sealed for &String {
    fn try_as_name(&self, _: Seal) -> Result<Cow<'_, HeaderName>, InvalidHeaderName> {
        HeaderName::from_str(self).map(Cow::Owned)
    }
}
impl AsHeaderName for &String {}

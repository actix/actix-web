use std::{
    fmt::{self, Write as _},
    str,
};

/// A wrapper for types used in header values where wildcard (`*`) items are allowed but the
/// underlying type does not support them.
///
/// For example, we use the `language-tags` crate for the [`AcceptLanguage`](super::AcceptLanguage)
/// typed header but it does parse `*` successfully. On the other hand, the `mime` crate, used for
/// [`Accept`](super::Accept), has first-party support for wildcard items so this wrapper is not
/// used in those header types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Hash)]
pub enum AnyOrSome<T> {
    /// A wildcard value.
    Any,

    /// A valid `T`.
    Item(T),
}

impl<T> AnyOrSome<T> {
    /// Returns true if item is wildcard (`*`) variant.
    pub fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    /// Returns true if item is a valid item (`T`) variant.
    pub fn is_item(&self) -> bool {
        matches!(self, Self::Item(_))
    }

    /// Returns reference to value in `Item` variant, if it is set.
    pub fn item(&self) -> Option<&T> {
        match self {
            AnyOrSome::Item(ref item) => Some(item),
            AnyOrSome::Any => None,
        }
    }

    /// Consumes the container, returning the value in the `Item` variant, if it is set.
    pub fn into_item(self) -> Option<T> {
        match self {
            AnyOrSome::Item(item) => Some(item),
            AnyOrSome::Any => None,
        }
    }
}

impl<T: fmt::Display> fmt::Display for AnyOrSome<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnyOrSome::Any => f.write_char('*'),
            AnyOrSome::Item(item) => fmt::Display::fmt(item, f),
        }
    }
}

impl<T: str::FromStr> str::FromStr for AnyOrSome<T> {
    type Err = T::Err;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "*" => Ok(Self::Any),
            other => other.parse().map(AnyOrSome::Item),
        }
    }
}

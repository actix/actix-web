use std::{
    fmt::{self, Write as _},
    str,
};

/// A wrapper for types used in header values where wildcard (`*`) items are allowed but the
/// underlying type does not support them.
///
/// For example, we use the `language-tags` crate for the [`AcceptLanguage`](super::AcceptLanguage)
/// typed header but it does not parse `*` successfully. On the other hand, the `mime` crate, used
/// for [`Accept`](super::Accept), has first-party support for wildcard items so this wrapper is not
/// used in those header types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Hash)]
pub enum Preference<T> {
    /// A wildcard value.
    Any,

    /// A valid `T`.
    Specific(T),
}

impl<T> Preference<T> {
    /// Returns true if preference is the any/wildcard (`*`) value.
    pub fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    /// Returns true if preference is the specific item (`T`) variant.
    pub fn is_specific(&self) -> bool {
        matches!(self, Self::Specific(_))
    }

    /// Returns reference to value in `Specific` variant, if it is set.
    pub fn item(&self) -> Option<&T> {
        match self {
            Preference::Specific(ref item) => Some(item),
            Preference::Any => None,
        }
    }

    /// Consumes the container, returning the value in the `Specific` variant, if it is set.
    pub fn into_item(self) -> Option<T> {
        match self {
            Preference::Specific(item) => Some(item),
            Preference::Any => None,
        }
    }
}

impl<T: fmt::Display> fmt::Display for Preference<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Preference::Any => f.write_char('*'),
            Preference::Specific(item) => fmt::Display::fmt(item, f),
        }
    }
}

impl<T: str::FromStr> str::FromStr for Preference<T> {
    type Err = T::Err;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "*" => Ok(Self::Any),
            other => other.parse().map(Preference::Specific),
        }
    }
}

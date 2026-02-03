//! Abstraction over `regex` and `regex-lite` depending on whether we have `unicode` crate feature
//! enabled.

use cfg_if::cfg_if;
#[cfg(feature = "unicode")]
pub(crate) use regex::{escape, Regex};
#[cfg(not(feature = "unicode"))]
pub(crate) use regex_lite::{escape, Regex};

#[cfg(feature = "unicode")]
#[derive(Debug, Clone)]
pub(crate) struct RegexSet(regex::RegexSet);

#[cfg(not(feature = "unicode"))]
#[derive(Debug, Clone)]
pub(crate) struct RegexSet(Vec<regex_lite::Regex>);

impl RegexSet {
    /// Create a new regex set.
    ///
    /// # Panics
    ///
    /// Panics if any path patterns are malformed.
    pub(crate) fn new(re_set: Vec<String>) -> Self {
        cfg_if! {
            if #[cfg(feature = "unicode")] {
                Self(regex::RegexSet::new(re_set).unwrap())
            } else {
                Self(re_set.iter().map(|re| Regex::new(re).unwrap()).collect())
            }
        }
    }

    /// Create a new empty regex set.
    pub(crate) fn empty() -> Self {
        cfg_if! {
            if #[cfg(feature = "unicode")] {
                Self(regex::RegexSet::empty())
            } else {
                Self(Vec::new())
            }
        }
    }

    /// Returns true if regex set matches `path`.
    pub(crate) fn is_match(&self, path: &str) -> bool {
        cfg_if! {
            if #[cfg(feature = "unicode")] {
                self.0.is_match(path)
            } else {
                self.0.iter().any(|re| re.is_match(path))
            }
        }
    }

    /// Returns index within `path` of first match.
    pub(crate) fn first_match_idx(&self, path: &str) -> Option<usize> {
        cfg_if! {
            if #[cfg(feature = "unicode")] {
                self.0.matches(path).into_iter().next()
            } else {
                Some(self.0.iter().enumerate().find(|(_, re)| re.is_match(path))?.0)
            }
        }
    }
}

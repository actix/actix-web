use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};

use super::Cookie;

/// A `DeltaCookie` is a helper structure used in a cookie jar. It wraps a
/// `Cookie` so that it can be hashed and compared purely by name. It further
/// records whether the wrapped cookie is a "removal" cookie, that is, a cookie
/// that when sent to the client removes the named cookie on the client's
/// machine.
#[derive(Clone, Debug)]
pub struct DeltaCookie {
    pub cookie: Cookie<'static>,
    pub removed: bool,
}

impl DeltaCookie {
    /// Create a new `DeltaCookie` that is being added to a jar.
    #[inline]
    pub fn added(cookie: Cookie<'static>) -> DeltaCookie {
        DeltaCookie {
            cookie,
            removed: false,
        }
    }

    /// Create a new `DeltaCookie` that is being removed from a jar. The
    /// `cookie` should be a "removal" cookie.
    #[inline]
    pub fn removed(cookie: Cookie<'static>) -> DeltaCookie {
        DeltaCookie {
            cookie,
            removed: true,
        }
    }
}

impl Deref for DeltaCookie {
    type Target = Cookie<'static>;

    fn deref(&self) -> &Cookie<'static> {
        &self.cookie
    }
}

impl DerefMut for DeltaCookie {
    fn deref_mut(&mut self) -> &mut Cookie<'static> {
        &mut self.cookie
    }
}

impl PartialEq for DeltaCookie {
    fn eq(&self, other: &DeltaCookie) -> bool {
        self.name() == other.name()
    }
}

impl Eq for DeltaCookie {}

impl Hash for DeltaCookie {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name().hash(state);
    }
}

impl Borrow<str> for DeltaCookie {
    fn borrow(&self) -> &str {
        self.name()
    }
}

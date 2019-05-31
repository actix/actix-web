use std::any::{Any, TypeId};
use std::fmt;

use hashbrown::HashMap;
use hashbrown::hash_map::{Keys, Values};
use hashbrown::hash_map::IntoIter;

#[derive(Default)]
/// A type map of request extensions.
pub struct Extensions {
    map: HashMap<TypeId, Box<Any>>,
}

impl Extensions {
    /// Create an empty `Extensions`.
    #[inline]
    pub fn new() -> Extensions {
        Extensions {
            map: HashMap::default(),
        }
    }

    /// Create `Extensions` with the provided content
    #[inline]
    pub fn from(e: Extensions) -> Extensions{
        Extensions {
            map: HashMap::from(e.map)
        }
    }

    /// Insert a type into this `Extensions`.
    ///
    /// If a extension of this type already existed, it will
    /// be returned.
    pub fn insert<T: 'static>(&mut self, val: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(val));
    }

    /// Check if container contains entry
    pub fn contains<T: 'static>(&self) -> bool {
        self.map.get(&TypeId::of::<T>()).is_some()
    }

    /// Get a reference to a type previously inserted on this `Extensions`.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|boxed| (&**boxed as &(Any + 'static)).downcast_ref())
    }

    /// Get a mutable reference to a type previously inserted on this `Extensions`.
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| (&mut **boxed as &mut (Any + 'static)).downcast_mut())
    }

    /// Remove a type from this `Extensions`.
    ///
    /// If a extension of this type existed, it will be returned.
    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map.remove(&TypeId::of::<T>()).and_then(|boxed| {
            (boxed as Box<Any + 'static>)
                .downcast()
                .ok()
                .map(|boxed| *boxed)
        })
    }

    /// Clear the `Extensions` of all inserted extensions.
    #[inline]
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn into_iter(&self) -> IntoIter<TypeId, Box<Any>> {
        self.map.into_iter()
    }

    pub fn keys(& self) -> Keys<TypeId, Box<Any>> {
        self.map.keys()
    }

    pub fn values(& self) -> Values<TypeId, Box<Any>> {
        self.map.values()
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Extensions").finish()
    }
}

#[test]
fn test_extensions() {
    #[derive(Debug, PartialEq)]
    struct MyType(i32);

    let mut extensions = Extensions::new();

    extensions.insert(5i32);
    extensions.insert(MyType(10));

    assert_eq!(extensions.get(), Some(&5i32));
    assert_eq!(extensions.get_mut(), Some(&mut 5i32));

    assert_eq!(extensions.remove::<i32>(), Some(5i32));
    assert!(extensions.get::<i32>().is_none());

    assert_eq!(extensions.get::<bool>(), None);
    assert_eq!(extensions.get(), Some(&MyType(10)));
}

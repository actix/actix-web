use std::any::{Any, TypeId};
use std::{fmt, mem};

use fxhash::FxHashMap;

#[derive(Default)]
/// A type map of request extensions.
pub struct Extensions {
    /// Use FxHasher with a std HashMap with for faster
    /// lookups on the small `TypeId` (u64 equivalent) keys.
    map: FxHashMap<TypeId, Box<dyn Any>>,
}

impl Extensions {
    /// Create an empty `Extensions`.
    #[inline]
    pub fn new() -> Extensions {
        Extensions {
            map: FxHashMap::default(),
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
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Get a reference to a type previously inserted on this `Extensions`.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref())
    }

    /// Get a mutable reference to a type previously inserted on this `Extensions`.
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut())
    }

    /// Remove a type from this `Extensions`.
    ///
    /// If a extension of this type existed, it will be returned.
    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast().ok().map(|boxed| *boxed))
    }

    /// Clear the `Extensions` of all inserted extensions.
    #[inline]
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Extends self with the items from another `Extensions`.
    pub fn extend(&mut self, other: Extensions) {
        self.map.extend(other.map);
    }

    /// Sets (or overrides) items from `other` into this map.
    pub(crate) fn drain_from(&mut self, other: &mut Self) {
        self.map.extend(mem::take(&mut other.map));
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove() {
        let mut map = Extensions::new();

        map.insert::<i8>(123);
        assert!(map.get::<i8>().is_some());

        map.remove::<i8>();
        assert!(map.get::<i8>().is_none());
    }

    #[test]
    fn test_clear() {
        let mut map = Extensions::new();

        map.insert::<i8>(8);
        map.insert::<i16>(16);
        map.insert::<i32>(32);

        assert!(map.contains::<i8>());
        assert!(map.contains::<i16>());
        assert!(map.contains::<i32>());

        map.clear();

        assert!(!map.contains::<i8>());
        assert!(!map.contains::<i16>());
        assert!(!map.contains::<i32>());

        map.insert::<i8>(10);
        assert_eq!(*map.get::<i8>().unwrap(), 10);
    }

    #[test]
    fn test_integers() {
        let mut map = Extensions::new();

        map.insert::<i8>(8);
        map.insert::<i16>(16);
        map.insert::<i32>(32);
        map.insert::<i64>(64);
        map.insert::<i128>(128);
        map.insert::<u8>(8);
        map.insert::<u16>(16);
        map.insert::<u32>(32);
        map.insert::<u64>(64);
        map.insert::<u128>(128);
        assert!(map.get::<i8>().is_some());
        assert!(map.get::<i16>().is_some());
        assert!(map.get::<i32>().is_some());
        assert!(map.get::<i64>().is_some());
        assert!(map.get::<i128>().is_some());
        assert!(map.get::<u8>().is_some());
        assert!(map.get::<u16>().is_some());
        assert!(map.get::<u32>().is_some());
        assert!(map.get::<u64>().is_some());
        assert!(map.get::<u128>().is_some());
    }

    #[test]
    fn test_composition() {
        struct Magi<T>(pub T);

        struct Madoka {
            pub god: bool,
        }

        struct Homura {
            pub attempts: usize,
        }

        struct Mami {
            pub guns: usize,
        }

        let mut map = Extensions::new();

        map.insert(Magi(Madoka { god: false }));
        map.insert(Magi(Homura { attempts: 0 }));
        map.insert(Magi(Mami { guns: 999 }));

        assert!(!map.get::<Magi<Madoka>>().unwrap().0.god);
        assert_eq!(0, map.get::<Magi<Homura>>().unwrap().0.attempts);
        assert_eq!(999, map.get::<Magi<Mami>>().unwrap().0.guns);
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

    #[test]
    fn test_extend() {
        #[derive(Debug, PartialEq)]
        struct MyType(i32);

        let mut extensions = Extensions::new();

        extensions.insert(5i32);
        extensions.insert(MyType(10));

        let mut other = Extensions::new();

        other.insert(15i32);
        other.insert(20u8);

        extensions.extend(other);

        assert_eq!(extensions.get(), Some(&15i32));
        assert_eq!(extensions.get_mut(), Some(&mut 15i32));

        assert_eq!(extensions.remove::<i32>(), Some(15i32));
        assert!(extensions.get::<i32>().is_none());

        assert_eq!(extensions.get::<bool>(), None);
        assert_eq!(extensions.get(), Some(&MyType(10)));

        assert_eq!(extensions.get(), Some(&20u8));
        assert_eq!(extensions.get_mut(), Some(&mut 20u8));
    }

    #[test]
    fn test_drain_from() {
        let mut ext = Extensions::new();
        ext.insert(2isize);

        let mut more_ext = Extensions::new();

        more_ext.insert(5isize);
        more_ext.insert(5usize);

        assert_eq!(ext.get::<isize>(), Some(&2isize));
        assert_eq!(ext.get::<usize>(), None);
        assert_eq!(more_ext.get::<isize>(), Some(&5isize));
        assert_eq!(more_ext.get::<usize>(), Some(&5usize));

        ext.drain_from(&mut more_ext);

        assert_eq!(ext.get::<isize>(), Some(&5isize));
        assert_eq!(ext.get::<usize>(), Some(&5usize));
        assert_eq!(more_ext.get::<isize>(), None);
        assert_eq!(more_ext.get::<usize>(), None);
    }
}

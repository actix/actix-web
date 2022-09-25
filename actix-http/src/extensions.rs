use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
    hash::{BuildHasherDefault, Hasher},
};

/// A hasher for `TypeId`s that takes advantage of its known characteristics.
///
/// Author of `anymap` crate has done research on the topic:
/// https://github.com/chris-morgan/anymap/blob/2e9a5704/src/lib.rs#L599
#[derive(Debug, Default)]
struct NoOpHasher(u64);

impl Hasher for NoOpHasher {
    fn write(&mut self, _bytes: &[u8]) {
        unimplemented!("This NoOpHasher can only handle u64s")
    }

    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// A type map for request extensions.
///
/// All entries into this map must be owned types (or static references).
#[derive(Default)]
pub struct Extensions {
    /// Use AHasher with a std HashMap with for faster lookups on the small `TypeId` keys.
    map: HashMap<TypeId, Box<dyn Any>, BuildHasherDefault<NoOpHasher>>,
}

impl Extensions {
    /// Creates an empty `Extensions`.
    #[inline]
    pub fn new() -> Extensions {
        Extensions {
            map: HashMap::default(),
        }
    }

    /// Insert an item into the map.
    ///
    /// If an item of this type was already stored, it will be replaced and returned.
    ///
    /// ```
    /// # use actix_http::Extensions;
    /// let mut map = Extensions::new();
    /// assert_eq!(map.insert(""), None);
    /// assert_eq!(map.insert(1u32), None);
    /// assert_eq!(map.insert(2u32), Some(1u32));
    /// assert_eq!(*map.get::<u32>().unwrap(), 2u32);
    /// ```
    pub fn insert<T: 'static>(&mut self, val: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(val))
            .and_then(downcast_owned)
    }

    /// Check if map contains an item of a given type.
    ///
    /// ```
    /// # use actix_http::Extensions;
    /// let mut map = Extensions::new();
    /// assert!(!map.contains::<u32>());
    ///
    /// assert_eq!(map.insert(1u32), None);
    /// assert!(map.contains::<u32>());
    /// ```
    pub fn contains<T: 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Get a reference to an item of a given type.
    ///
    /// ```
    /// # use actix_http::Extensions;
    /// let mut map = Extensions::new();
    /// map.insert(1u32);
    /// assert_eq!(map.get::<u32>(), Some(&1u32));
    /// ```
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref())
    }

    /// Get a mutable reference to an item of a given type.
    ///
    /// ```
    /// # use actix_http::Extensions;
    /// let mut map = Extensions::new();
    /// map.insert(1u32);
    /// assert_eq!(map.get_mut::<u32>(), Some(&mut 1u32));
    /// ```
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut())
    }

    /// Remove an item from the map of a given type.
    ///
    /// If an item of this type was already stored, it will be returned.
    ///
    /// ```
    /// # use actix_http::Extensions;
    /// let mut map = Extensions::new();
    ///
    /// map.insert(1u32);
    /// assert_eq!(map.get::<u32>(), Some(&1u32));
    ///
    /// assert_eq!(map.remove::<u32>(), Some(1u32));
    /// assert!(!map.contains::<u32>());
    /// ```
    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map.remove(&TypeId::of::<T>()).and_then(downcast_owned)
    }

    /// Clear the `Extensions` of all inserted extensions.
    ///
    /// ```
    /// # use actix_http::Extensions;
    /// let mut map = Extensions::new();
    ///
    /// map.insert(1u32);
    /// assert!(map.contains::<u32>());
    ///
    /// map.clear();
    /// assert!(!map.contains::<u32>());
    /// ```
    #[inline]
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Extends self with the items from another `Extensions`.
    pub fn extend(&mut self, other: Extensions) {
        self.map.extend(other.map);
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions").finish()
    }
}

fn downcast_owned<T: 'static>(boxed: Box<dyn Any>) -> Option<T> {
    boxed.downcast().ok().map(|boxed| *boxed)
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
        static A: u32 = 8;

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
        map.insert::<&'static u32>(&A);
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
        assert!(map.get::<&'static u32>().is_some());
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
}

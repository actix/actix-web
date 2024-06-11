macro_rules! downcast_get_type_id {
    () => {
        /// A helper method to get the type ID of the type
        /// this trait is implemented on.
        /// This method is unsafe to *implement*, since `downcast_ref` relies
        /// on the returned `TypeId` to perform a cast.
        ///
        /// Unfortunately, Rust has no notion of a trait method that is
        /// unsafe to implement (marking it as `unsafe` makes it unsafe
        /// to *call*). As a workaround, we require this method
        /// to return a private type along with the `TypeId`. This
        /// private type (`PrivateHelper`) has a private constructor,
        /// making it impossible for safe code to construct outside of
        /// this module. This ensures that safe code cannot violate
        /// type-safety by implementing this method.
        ///
        /// We also take `PrivateHelper` as a parameter, to ensure that
        /// safe code cannot obtain a `PrivateHelper` instance by
        /// delegating to an existing implementation of `__private_get_type_id__`
        #[doc(hidden)]
        #[allow(dead_code)]
        fn __private_get_type_id__(&self, _: PrivateHelper) -> (std::any::TypeId, PrivateHelper)
        where
            Self: 'static,
        {
            (std::any::TypeId::of::<Self>(), PrivateHelper(()))
        }
    };
}

// Generate implementation for dyn $name
macro_rules! downcast_dyn {
    ($name:ident) => {
        /// A struct with a private constructor, for use with
        /// `__private_get_type_id__`. Its single field is private,
        /// ensuring that it can only be constructed from this module
        #[doc(hidden)]
        #[allow(dead_code)]
        pub struct PrivateHelper(());

        impl dyn $name + 'static {
            /// Downcasts generic body to a specific type.
            #[allow(dead_code)]
            pub fn downcast_ref<T: $name + 'static>(&self) -> Option<&T> {
                if self.__private_get_type_id__(PrivateHelper(())).0 == std::any::TypeId::of::<T>()
                {
                    // SAFETY: external crates cannot override the default
                    // implementation of `__private_get_type_id__`, since
                    // it requires returning a private type. We can therefore
                    // rely on the returned `TypeId`, which ensures that this
                    // case is correct.
                    unsafe { Some(&*(self as *const dyn $name as *const T)) }
                } else {
                    None
                }
            }

            /// Downcasts a generic body to a mutable specific type.
            #[allow(dead_code)]
            pub fn downcast_mut<T: $name + 'static>(&mut self) -> Option<&mut T> {
                if self.__private_get_type_id__(PrivateHelper(())).0 == std::any::TypeId::of::<T>()
                {
                    // SAFETY: external crates cannot override the default
                    // implementation of `__private_get_type_id__`, since
                    // it requires returning a private type. We can therefore
                    // rely on the returned `TypeId`, which ensures that this
                    // case is correct.
                    unsafe { Some(&mut *(self as *const dyn $name as *const T as *mut T)) }
                } else {
                    None
                }
            }
        }
    };
}

pub(crate) use downcast_dyn;
pub(crate) use downcast_get_type_id;

#[cfg(test)]
mod tests {
    #![allow(clippy::upper_case_acronyms)]

    trait MB {
        downcast_get_type_id!();
    }

    downcast_dyn!(MB);

    impl MB for String {}
    impl MB for () {}

    #[actix_rt::test]
    async fn test_any_casting() {
        let mut body = String::from("hello cast");
        let resp_body: &mut dyn MB = &mut body;
        let body = resp_body.downcast_ref::<String>().unwrap();
        assert_eq!(body, "hello cast");
        let body = resp_body.downcast_mut::<String>().unwrap();
        body.push('!');
        let body = resp_body.downcast_ref::<String>().unwrap();
        assert_eq!(body, "hello cast!");
        let not_body = resp_body.downcast_ref::<()>();
        assert!(not_body.is_none());
    }
}

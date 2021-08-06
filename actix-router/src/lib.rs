//! Resource path matching and router.

#![deny(rust_2018_idioms, nonstandard_style)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

mod de;
mod path;
mod resource;
mod router;

pub use self::de::PathDeserializer;
pub use self::path::Path;
pub use self::resource::ResourceDef;
pub use self::router::{ResourceInfo, Router, RouterBuilder};

// TODO: this trait is necessary, document it
// see impl Resource for ServiceRequest
pub trait Resource<T: ResourcePath> {
    fn resource_path(&mut self) -> &mut Path<T>;
}

pub trait ResourcePath {
    fn path(&self) -> &str;
}

impl ResourcePath for String {
    fn path(&self) -> &str {
        self.as_str()
    }
}

impl<'a> ResourcePath for &'a str {
    fn path(&self) -> &str {
        self
    }
}

impl ResourcePath for bytestring::ByteString {
    fn path(&self) -> &str {
        &*self
    }
}

/// One or many patterns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Patterns {
    Single(String),
    List(Vec<String>),
}

impl Patterns {
    pub fn is_empty(&self) -> bool {
        match self {
            Patterns::Single(_) => false,
            Patterns::List(pats) => pats.is_empty(),
        }
    }
}

/// Helper trait for type that could be converted to one or more path pattern.
pub trait IntoPatterns {
    fn patterns(&self) -> Patterns;
}

impl IntoPatterns for String {
    fn patterns(&self) -> Patterns {
        Patterns::Single(self.clone())
    }
}

impl<'a> IntoPatterns for &'a String {
    fn patterns(&self) -> Patterns {
        Patterns::Single((*self).clone())
    }
}

impl<'a> IntoPatterns for &'a str {
    fn patterns(&self) -> Patterns {
        Patterns::Single((*self).to_owned())
    }
}

impl IntoPatterns for bytestring::ByteString {
    fn patterns(&self) -> Patterns {
        Patterns::Single(self.to_string())
    }
}

impl IntoPatterns for Patterns {
    fn patterns(&self) -> Patterns {
        self.clone()
    }
}

impl<T: AsRef<str>> IntoPatterns for Vec<T> {
    fn patterns(&self) -> Patterns {
        let mut patterns = self.iter().map(|v| v.as_ref().to_owned());

        match patterns.size_hint() {
            (1, _) => Patterns::Single(patterns.next().unwrap()),
            _ => Patterns::List(patterns.collect()),
        }
    }
}

macro_rules! array_patterns_single (($tp:ty) => {
    impl IntoPatterns for [$tp; 1] {
        fn patterns(&self) -> Patterns {
            Patterns::Single(self[0].to_owned())
        }
    }
});

macro_rules! array_patterns_multiple (($tp:ty, $str_fn:expr, $($num:tt) +) => {
    // for each array length specified in $num
    $(
        impl IntoPatterns for [$tp; $num] {
            fn patterns(&self) -> Patterns {
                Patterns::List(self.iter().map($str_fn).collect())
            }
        }
    )+
});

array_patterns_single!(&str);
array_patterns_multiple!(&str, |&v| v.to_owned(), 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);

array_patterns_single!(String);
array_patterns_multiple!(String, |v| v.clone(), 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);

#[cfg(feature = "http")]
mod url;

#[cfg(feature = "http")]
pub use self::url::{Quoter, Url};

#[cfg(feature = "http")]
mod http_impls {
    use http::Uri;

    use super::ResourcePath;

    impl ResourcePath for Uri {
        fn path(&self) -> &str {
            self.path()
        }
    }
}

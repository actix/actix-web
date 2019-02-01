//! Resource path matching library.
mod de;
mod path;
mod pattern;
mod router;

pub use self::de::PathDeserializer;
pub use self::path::Path;
pub use self::pattern::Pattern;
pub use self::router::{ResourceInfo, Router, RouterBuilder};

pub trait RequestPath {
    fn path(&self) -> &str;
}

impl RequestPath for String {
    fn path(&self) -> &str {
        self.as_str()
    }
}

impl<'a> RequestPath for &'a str {
    fn path(&self) -> &str {
        self
    }
}

impl<T: AsRef<[u8]>> RequestPath for string::String<T> {
    fn path(&self) -> &str {
        &*self
    }
}

#[cfg(feature = "http")]
mod url;

#[cfg(feature = "http")]
pub use self::url::Url;

#[cfg(feature = "http")]
mod http_support {
    use super::RequestPath;
    use http::Uri;

    impl RequestPath for Uri {
        fn path(&self) -> &str {
            self.path()
        }
    }
}

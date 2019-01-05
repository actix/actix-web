//! Resource path matching library.
mod de;
mod path;
mod pattern;
mod router;

pub use self::de::PathDeserializer;
pub use self::path::Path;
pub use self::pattern::Pattern;
pub use self::router::{Router, RouterBuilder};

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

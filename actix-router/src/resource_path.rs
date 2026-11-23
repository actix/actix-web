use crate::Path;

/// Abstraction over types that can provide a mutable [`Path`] for routing.
///
/// This trait is used by the router to extract the request path in a uniform way across different
/// request types (e.g., Actix Web's `ServiceRequest`). Implementers return a mutable [`Path`]
/// wrapper so routing can read and potentially normalize/parse the path without requiring the
/// original request type.
pub trait Resource {
    /// Type of resource's path returned in `resource_path`.
    type Path: ResourcePath;

    /// Returns a mutable reference to the path wrapper used by the router.
    fn resource_path(&mut self) -> &mut Path<Self::Path>;
}

pub trait ResourcePath {
    fn path(&self) -> &str;
}

impl ResourcePath for String {
    fn path(&self) -> &str {
        self.as_str()
    }
}

impl ResourcePath for &str {
    fn path(&self) -> &str {
        self
    }
}

impl ResourcePath for bytestring::ByteString {
    fn path(&self) -> &str {
        self
    }
}

#[cfg(feature = "http")]
impl ResourcePath for http::Uri {
    fn path(&self) -> &str {
        self.path()
    }
}

#[cfg(test)]
mod tests {
    use bytestring::ByteString;

    use super::*;

    #[test]
    fn resource_path_implementations_expose_their_path() {
        let owned = "/owned".to_owned();
        assert_eq!(<String as ResourcePath>::path(&owned), "/owned");

        let borrowed = "/borrowed";
        assert_eq!(<&str as ResourcePath>::path(&borrowed), "/borrowed");

        let bytes = ByteString::from("/bytes");
        assert_eq!(<ByteString as ResourcePath>::path(&bytes), "/bytes");
    }

    #[cfg(feature = "http")]
    #[test]
    fn uri_resource_path_ignores_queries() {
        let uri = http::Uri::from_static("/resource?query=value");

        assert_eq!(<http::Uri as ResourcePath>::path(&uri), "/resource");
    }
}

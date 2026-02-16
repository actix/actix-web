use crate::Path;

/// Abstraction over types that can provide a mutable [`Path`] for routing.
///
/// This trait is used by the router to extract the request path in a uniform way across different
/// request types (e.g., Actix Web's `ServiceRequest`). Implementors return a mutable [`Path`]
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

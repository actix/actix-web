use crate::Path;

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

#[cfg(feature = "http")]
impl ResourcePath for http::Uri {
    fn path(&self) -> &str {
        self.path()
    }
}

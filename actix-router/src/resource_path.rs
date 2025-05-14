use std::mem;

use crate::resource::ResourceMatchInfo;
use crate::Path;

// TODO: this trait is necessary, document it
// see impl Resource for ServiceRequest
pub trait Resource {
    /// Type of resource's path returned in `resource_path`.
    type Path: ResourcePath;

    fn resource_path(&mut self) -> &mut Path<Self::Path>;

    fn resolve_path(&mut self, match_info: ResourceMatchInfo<'_>) {
        let path = self.resource_path();
        match match_info {
            ResourceMatchInfo::Static { matched_len } => {
                path.skip(matched_len);
            }
            ResourceMatchInfo::Dynamic {
                matched_len,
                matched_vars,
                mut segments,
            } => {
                for i in 0..matched_vars.len() {
                    path.add(matched_vars[i], mem::take(&mut segments[i]));
                }

                path.skip(matched_len);
            }
        }
    }
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

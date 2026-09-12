use std::{
    fmt::{self, Write},
    fs::DirEntry,
    io,
    path::{Path, PathBuf},
    rc::Rc,
};

use actix_web::{dev::ServiceResponse, HttpRequest, HttpResponse};
use percent_encoding::{utf8_percent_encode, CONTROLS};
use v_htmlescape::escape_fmt;

use crate::PathFilter;

/// A directory; responds with the generated directory listing.
pub struct Directory {
    /// Base directory.
    pub base: PathBuf,

    /// Path of subdirectory to generate listing for.
    pub path: PathBuf,

    path_filter: Option<Rc<PathFilter>>,
}

impl fmt::Debug for Directory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Directory")
            .field("base", &self.base)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Directory {
    /// Create a new directory
    pub fn new(base: PathBuf, path: PathBuf) -> Directory {
        Directory {
            base,
            path,
            path_filter: None,
        }
    }

    pub(crate) fn with_path_filter(mut self, path_filter: Option<Rc<PathFilter>>) -> Self {
        self.path_filter = path_filter;
        self
    }

    /// Is this entry visible from this directory?
    ///
    /// Hidden files (names starting with `.`) are excluded. This does **not** apply
    /// [`crate::Files::path_filter`]; use [`Self::is_visible_for`] when rendering listings.
    pub fn is_visible(&self, entry: &io::Result<DirEntry>) -> bool {
        if let Ok(ref entry) = *entry {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with('.') {
                    return false;
                }
            }
            if let Ok(ref md) = entry.metadata() {
                let ft = md.file_type();
                return ft.is_dir() || ft.is_file() || ft.is_symlink();
            }
        }
        false
    }

    /// Returns `true` if `entry` should appear in a directory listing for `req`.
    ///
    /// Applies [`Self::is_visible`] and any [`crate::Files::path_filter`] attached by the files
    /// service. The path passed to the filter is relative to [`Self::base`].
    pub fn is_visible_for(&self, entry: &io::Result<DirEntry>, req: &HttpRequest) -> bool {
        if !self.is_visible(entry) {
            return false;
        }

        let Some(filter) = &self.path_filter else {
            return true;
        };

        let Ok(entry) = entry else {
            return false;
        };

        match entry.path().strip_prefix(&self.base) {
            Ok(rel) => filter(rel, req.head()),
            Err(_) => false,
        }
    }
}

pub(crate) type DirectoryRenderer =
    dyn Fn(&Directory, &HttpRequest) -> Result<ServiceResponse, io::Error>;

/// Returns percent encoded file URL path.
macro_rules! encode_file_url {
    ($path:ident) => {
        utf8_percent_encode(&$path, CONTROLS)
    };
}

/// Returns HTML entity encoded formatter.
///
/// ```plain
/// " => &quot;
/// & => &amp;
/// ' => &#x27;
/// < => &lt;
/// > => &gt;
/// / => &#x2f;
/// ```
macro_rules! encode_file_name {
    ($entry:ident) => {
        escape_fmt(&$entry.file_name().to_string_lossy())
    };
}

pub(crate) fn directory_listing(
    dir: &Directory,
    req: &HttpRequest,
) -> Result<ServiceResponse, io::Error> {
    let index_of = format!("Index of {}", req.path());
    let mut body = String::new();
    let base = Path::new(req.path());

    for entry in dir.path.read_dir()? {
        if dir.is_visible_for(&entry, req) {
            let entry = entry.unwrap();
            let p = match entry.path().strip_prefix(&dir.path) {
                Ok(p) if cfg!(windows) => base.join(p).to_string_lossy().replace('\\', "/"),
                Ok(p) => base.join(p).to_string_lossy().into_owned(),
                Err(_) => continue,
            };

            // if file is a directory, add '/' to the end of the name
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    let _ = write!(
                        body,
                        "<li><a href=\"{}\">{}/</a></li>",
                        encode_file_url!(p),
                        encode_file_name!(entry),
                    );
                } else {
                    let _ = write!(
                        body,
                        "<li><a href=\"{}\">{}</a></li>",
                        encode_file_url!(p),
                        encode_file_name!(entry),
                    );
                }
            } else {
                continue;
            }
        }
    }

    let html = format!(
        "<html>\
         <head><title>{}</title></head>\
         <body><h1>{}</h1>\
         <ul>\
         {}\
         </ul></body>\n</html>",
        index_of, index_of, body
    );
    Ok(ServiceResponse::new(
        req.clone(),
        HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(html),
    ))
}

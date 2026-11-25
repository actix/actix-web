use std::{
    fmt::Write,
    fs::DirEntry,
    io,
    path::{Path, PathBuf},
};

use actix_web::{dev::ServiceResponse, HttpRequest, HttpResponse};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use v_htmlescape::escape_fmt;

/// A directory; responds with the generated directory listing.
#[derive(Debug)]
pub struct Directory {
    /// Base directory.
    pub base: PathBuf,

    /// Path of subdirectory to generate listing for.
    pub path: PathBuf,
}

impl Directory {
    /// Create a new directory
    pub fn new(base: PathBuf, path: PathBuf) -> Directory {
        Directory { base, path }
    }

    /// Is this entry visible from this directory?
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

    /// Returns the visible entries of this directory, in `read_dir` order.
    ///
    /// Hidden entries are skipped, using the same rules as [`Directory::is_visible()`]. Entries
    /// whose metadata can't be read, or whose path can't be made relative to `self.path`, are
    /// skipped too. URLs are built from `req.path()`.
    pub fn entries(&self, req: &HttpRequest) -> io::Result<Vec<DirectoryEntry>> {
        let base = Path::new(req.path());
        let mut entries = Vec::new();

        for entry in self.path.read_dir()? {
            if !self.is_visible(&entry) {
                continue;
            }
            let entry = entry.unwrap();
            let p = match entry.path().strip_prefix(&self.path) {
                Ok(p) if cfg!(windows) => base.join(p).to_string_lossy().replace('\\', "/"),
                Ok(p) => base.join(p).to_string_lossy().into_owned(),
                Err(_) => continue,
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let url = p
                .split('/')
                .map(|segment| utf8_percent_encode(segment, PATH_SEGMENT).to_string())
                .collect::<Vec<_>>()
                .join("/");

            entries.push(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                url,
                is_dir: metadata.is_dir(),
                metadata,
            });
        }

        Ok(entries)
    }
}

/// A visible entry of a [`Directory`], as returned by [`Directory::entries()`].
#[derive(Debug)]
#[non_exhaustive]
pub struct DirectoryEntry {
    /// File name (lossy UTF-8). Not HTML-escaped.
    pub name: String,
    /// Percent-encoded URL path of the entry (request path joined with the file name).
    /// Not HTML-escaped; escape it before putting it in markup.
    pub url: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
    /// Metadata of the entry.
    pub metadata: std::fs::Metadata,
}

pub(crate) type DirectoryRenderer =
    dyn Fn(&Directory, &HttpRequest) -> Result<ServiceResponse, io::Error>;

// Leave only RFC 3986 unreserved characters unencoded within each path segment.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) fn directory_listing(
    dir: &Directory,
    req: &HttpRequest,
) -> Result<ServiceResponse, io::Error> {
    let index_of = format!("Index of {}", req.path());
    let mut body = String::new();

    for entry in dir.entries(req)? {
        // Slashes are safe in a quoted attribute. Keep path separators literal.
        let href = escape_fmt(&entry.url).to_string().replace("&#x2f;", "/");
        // if file is a directory, add '/' to the end of the name
        let slash = if entry.is_dir { "/" } else { "" };
        let _ = write!(
            body,
            "<li><a href=\"{}\">{}{}</a></li>",
            href,
            escape_fmt(&entry.name),
            slash,
        );
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

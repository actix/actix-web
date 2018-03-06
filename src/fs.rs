//! Static files support.

// //! TODO: needs to re-implement actual files handling, current impl blocks
use std::io;
use std::io::Read;
use std::fmt::Write;
use std::fs::{File, DirEntry, Metadata};
use std::path::{Path, PathBuf};
use std::ops::{Deref, DerefMut};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use http::{Method, StatusCode};
use mime_guess::get_mime_type;

use header;
use param::FromParam;
use handler::{Handler, Responder};
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use httpcodes::{HttpOk, HttpFound, HttpMethodNotAllowed};

/// A file with an associated name; responds with the Content-Type based on the
/// file extension.
#[derive(Debug)]
pub struct NamedFile {
    path: PathBuf,
    file: File,
    md: Metadata,
    modified: Option<SystemTime>,
}

impl NamedFile {
    /// Attempts to open a file in read-only mode.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use actix_web::fs::NamedFile;
    ///
    /// let file = NamedFile::open("foo.txt");
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<NamedFile> {
        let file = File::open(path.as_ref())?;
        let md = file.metadata()?;
        let path = path.as_ref().to_path_buf();
        let modified = md.modified().ok();
        Ok(NamedFile{path, file, md, modified})
    }

    /// Returns reference to the underlying `File` object.
    #[inline]
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Retrieve the path of this file.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use std::io;
    /// use actix_web::fs::NamedFile;
    ///
    /// # fn path() -> io::Result<()> {
    /// let file = NamedFile::open("test.txt")?;
    /// assert_eq!(file.path().as_os_str(), "foo.txt");
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn etag(&self) -> Option<header::EntityTag> {
        // This etag format is similar to Apache's.
        self.modified.as_ref().map(|mtime| {
            let ino = {
                #[cfg(unix)]
                { self.md.ino() }
                #[cfg(not(unix))]
                { 0 }
            };

            let dur = mtime.duration_since(UNIX_EPOCH)
                .expect("modification time must be after epoch");
            header::EntityTag::strong(
                format!("{:x}:{:x}:{:x}:{:x}",
                        ino, self.md.len(), dur.as_secs(),
                        dur.subsec_nanos()))
        })
    }

    fn last_modified(&self) -> Option<header::HttpDate> {
        self.modified.map(|mtime| mtime.into())
    }
}

impl Deref for NamedFile {
    type Target = File;

    fn deref(&self) -> &File {
        &self.file
    }
}

impl DerefMut for NamedFile {
    fn deref_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

/// Returns true if `req` has no `If-Match` header or one which matches `etag`.
fn any_match(etag: Option<&header::EntityTag>, req: &HttpRequest) -> bool {
    match req.get_header::<header::IfMatch>() {
        Err(_) | Ok(header::IfMatch::Any) => true,
        Ok(header::IfMatch::Items(ref items)) => {
            if let Some(some_etag) = etag {
                for item in items {
                    if item.strong_eq(some_etag) {
                        return true;
                    }
                }
            }
            false
        }
    }
}

/// Returns true if `req` doesn't have an `If-None-Match` header matching `req`.
fn none_match(etag: Option<&header::EntityTag>, req: &HttpRequest) -> bool {
    match req.get_header::<header::IfNoneMatch>() {
        Ok(header::IfNoneMatch::Any) => false,
        Ok(header::IfNoneMatch::Items(ref items)) => {
            if let Some(some_etag) = etag {
                for item in items {
                    if item.weak_eq(some_etag) {
                        return false;
                    }
                }
            }
            true
        }
        Err(_) => true,
    }
}


impl Responder for NamedFile {
    type Item = HttpResponse;
    type Error = io::Error;

    fn respond_to(mut self, req: HttpRequest) -> Result<HttpResponse, io::Error> {
        if *req.method() != Method::GET && *req.method() != Method::HEAD {
            return Ok(HttpMethodNotAllowed.build()
                      .header(header::http::CONTENT_TYPE, "text/plain")
                      .header(header::http::ALLOW, "GET, HEAD")
                      .body("This resource only supports GET and HEAD.").unwrap())
        }

        let etag = self.etag();
        let last_modified = self.last_modified();

        // check preconditions
        let precondition_failed = if !any_match(etag.as_ref(), &req) {
            true
        } else if let (Some(ref m), Ok(header::IfUnmodifiedSince(ref since))) =
            (last_modified, req.get_header())
        {
            m > since
        } else {
            false
        };

        // check last modified
        let not_modified = if !none_match(etag.as_ref(), &req) {
            true
        } else if let (Some(ref m), Ok(header::IfModifiedSince(ref since))) =
            (last_modified, req.get_header())
        {
            m <= since
        } else {
            false
        };

        let mut resp = HttpOk.build();

        resp
            .if_some(self.path().extension(), |ext, resp| {
                resp.set(header::ContentType(get_mime_type(&ext.to_string_lossy())));
            })
            .if_some(last_modified, |lm, resp| {resp.set(header::LastModified(lm));})
            .if_some(etag, |etag, resp| {resp.set(header::ETag(etag));});

        if precondition_failed {
            return Ok(resp.status(StatusCode::PRECONDITION_FAILED).finish().unwrap())
        } else if not_modified {
            return Ok(resp.status(StatusCode::NOT_MODIFIED).finish().unwrap())
        }

        resp.content_length(self.md.len());

        if *req.method() == Method::GET {
            let mut data = Vec::new();
            let _ = self.file.read_to_end(&mut data);
            Ok(resp.body(data).unwrap())
        } else {
            Ok(resp.finish().unwrap())
        }
    }
}

/// A directory; responds with the generated directory listing.
#[derive(Debug)]
pub struct Directory{
    base: PathBuf,
    path: PathBuf
}

impl Directory {
    pub fn new(base: PathBuf, path: PathBuf) -> Directory {
        Directory { base, path }
    }

    fn can_list(&self, entry: &io::Result<DirEntry>) -> bool {
        if let Ok(ref entry) = *entry {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with('.') {
                    return false
                }
            }
            if let Ok(ref md) = entry.metadata() {
                let ft = md.file_type();
                return ft.is_dir() || ft.is_file() || ft.is_symlink()
            }
        }
        false
    }
}

impl Responder for Directory {
    type Item = HttpResponse;
    type Error = io::Error;

    fn respond_to(self, req: HttpRequest) -> Result<HttpResponse, io::Error> {
        let index_of = format!("Index of {}", req.path());
        let mut body = String::new();
        let base = Path::new(req.path());

        for entry in self.path.read_dir()? {
            if self.can_list(&entry) {
                let entry = entry.unwrap();
                let p = match entry.path().strip_prefix(&self.path) {
                    Ok(p) => base.join(p),
                    Err(_) => continue
                };
                // show file url as relative to static path
                let file_url = format!("{}", p.to_string_lossy());

                // if file is a directory, add '/' to the end of the name
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_dir() {
                        let _ = write!(body, "<li><a href=\"{}\">{}/</a></li>",
                                       file_url, entry.file_name().to_string_lossy());
                    } else {
                        let _ = write!(body, "<li><a href=\"{}\">{}</a></li>",
                                       file_url, entry.file_name().to_string_lossy());
                    }
                } else {
                    continue
                }
            }
        }

        let html = format!("<html>\
                            <head><title>{}</title></head>\
                            <body><h1>{}</h1>\
                            <ul>\
                            {}\
                            </ul></body>\n</html>", index_of, index_of, body);
        Ok(HttpOk.build()
           .content_type("text/html; charset=utf-8")
           .body(html).unwrap())
    }
}

/// This enum represents all filesystem elements.
pub enum FilesystemElement {
    File(NamedFile),
    Directory(Directory),
    Redirect(HttpResponse),
}

impl Responder for FilesystemElement {
    type Item = HttpResponse;
    type Error = io::Error;

    fn respond_to(self, req: HttpRequest) -> Result<HttpResponse, io::Error> {
        match self {
            FilesystemElement::File(file) => file.respond_to(req),
            FilesystemElement::Directory(dir) => dir.respond_to(req),
            FilesystemElement::Redirect(resp) => Ok(resp),
        }
    }
}


/// Static files handling
///
/// `StaticFile` handler must be registered with `Application::handler()` method,
/// because `StaticFile` handler requires access sub-path information.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{fs, Application};
///
/// fn main() {
///     let app = Application::new()
///         .handler("/static", fs::StaticFiles::new(".", true))
///         .finish();
/// }
/// ```
pub struct StaticFiles {
    directory: PathBuf,
    accessible: bool,
    index: Option<String>,
    show_index: bool,
    _chunk_size: usize,
    _follow_symlinks: bool,
}

impl StaticFiles {
    /// Create new `StaticFiles` instance
    ///
    /// `dir` - base directory
    ///
    /// `index` - show index for directory
    pub fn new<T: Into<PathBuf>>(dir: T, index: bool) -> StaticFiles {
        let dir = dir.into();

        let (dir, access) = match dir.canonicalize() {
            Ok(dir) => {
                if dir.is_dir() {
                    (dir, true)
                } else {
                    warn!("Is not directory `{:?}`", dir);
                    (dir, false)
                }
            },
            Err(err) => {
                warn!("Static files directory `{:?}` error: {}", dir, err);
                (dir, false)
            }
        };

        StaticFiles {
            directory: dir,
            accessible: access,
            index: None,
            show_index: index,
            _chunk_size: 0,
            _follow_symlinks: false,
        }
    }

    /// Set index file
    ///
    /// Redirects to specific index file for directory "/" instead of
    /// showing files listing.
    pub fn index_file<T: Into<String>>(mut self, index: T) -> StaticFiles {
        self.index = Some(index.into());
        self
    }
}

impl<S> Handler<S> for StaticFiles {
    type Result = Result<FilesystemElement, io::Error>;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        if !self.accessible {
            Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
        } else {
            let path = if let Some(path) = req.match_info().get("tail") {
                path
            } else {
                return Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
            };

            let relpath = PathBuf::from_param(path)
                .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "not found"))?;

            // full filepath
            let path = self.directory.join(&relpath).canonicalize()?;

            if path.is_dir() {
                if let Some(ref redir_index) = self.index {
                    // TODO: Don't redirect, just return the index content.
                    // TODO: It'd be nice if there were a good usable URL manipulation library
                    let mut new_path: String = req.path().to_owned();
                    for el in relpath.iter() {
                        new_path.push_str(&el.to_string_lossy());
                        new_path.push('/');
                    }
                    new_path.push_str(redir_index);
                    Ok(FilesystemElement::Redirect(
                        HttpFound
                            .build()
                            .header::<_, &str>("LOCATION", &new_path)
                            .finish().unwrap()))
                } else if self.show_index {
                    Ok(FilesystemElement::Directory(Directory::new(self.directory.clone(), path)))
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
                }
            } else {
                Ok(FilesystemElement::File(NamedFile::open(path)?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test::TestRequest;
    use http::{header, Method, StatusCode};

    #[test]
    fn test_named_file() {
        assert!(NamedFile::open("test--").is_err());
        let mut file = NamedFile::open("Cargo.toml").unwrap();
        { file.file();
          let _f: &File = &file; }
        { let _f: &mut File = &mut file; }

        let resp = file.respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "text/x-toml")
    }

    #[test]
    fn test_named_file_not_allowed() {
        let req = TestRequest::default().method(Method::POST).finish();
        let file = NamedFile::open("Cargo.toml").unwrap();

        let resp = file.respond_to(req).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_static_files() {
        let mut st = StaticFiles::new(".", true);
        st.accessible = false;
        assert!(st.handle(HttpRequest::default()).is_err());

        st.accessible = true;
        st.show_index = false;
        assert!(st.handle(HttpRequest::default()).is_err());

        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "");

        st.show_index = true;
        let resp = st.handle(req).respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "text/html; charset=utf-8");
        assert!(resp.body().is_binary());
        assert!(format!("{:?}", resp.body()).contains("README.md"));
    }

    #[test]
    fn test_redirect_to_index() {
        let mut st = StaticFiles::new(".", false).index_file("index.html");
        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "guide");

        let resp = st.handle(req).respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/guide/index.html");

        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "guide/");

        let resp = st.handle(req).respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/guide/index.html");
    }

    #[test]
    fn test_redirect_to_index_nested() {
        let mut st = StaticFiles::new(".", false).index_file("Cargo.toml");
        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "examples/basics");

        let resp = st.handle(req).respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/examples/basics/Cargo.toml");
    }
}

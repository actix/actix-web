//! Static files support
use std::fmt::Write;
use std::fs::{DirEntry, File, Metadata};
use std::io::{Read, Seek};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{cmp, env, io};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use bytes::{BufMut, Bytes, BytesMut};
use futures::{Async, Future, Poll, Stream};
use futures_cpupool::{CpuFuture, CpuPool};
use mime;
use mime_guess::{get_mime_type, guess_mime_type};

use error::Error;
use handler::{AsyncResult, Handler, Responder, RouteHandler, WrapHandler};
use header;
use http::{ContentEncoding, HttpRange, Method, StatusCode};
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use param::FromParam;

/// Env variable for default cpu pool size for `StaticFiles`
const ENV_CPU_POOL_VAR: &str = "ACTIX_FS_POOL";

/// Return the MIME type associated with a filename extension (case-insensitive).
/// If `ext` is empty or no associated type for the extension was found, returns
/// the type `application/octet-stream`.
#[inline]
pub fn file_extension_to_mime(ext: &str) -> mime::Mime {
    get_mime_type(ext)
}

/// A file with an associated name; responds with the Content-Type based on the
/// file extension.
#[derive(Debug)]
pub struct NamedFile {
    path: PathBuf,
    file: File,
    md: Metadata,
    modified: Option<SystemTime>,
    cpu_pool: Option<CpuPool>,
    encoding: Option<ContentEncoding>,
    only_get: bool,
    status_code: StatusCode,
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
        let cpu_pool = None;
        let encoding = None;
        Ok(NamedFile {
            path,
            file,
            md,
            modified,
            cpu_pool,
            encoding,
            only_get: false,
            status_code: StatusCode::OK,
        })
    }

    /// Allow only GET and HEAD methods
    #[inline]
    pub fn only_get(mut self) -> Self {
        self.only_get = true;
        self
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

    /// Set `CpuPool` to use
    #[inline]
    pub fn set_cpu_pool(mut self, cpu_pool: CpuPool) -> Self {
        self.cpu_pool = Some(cpu_pool);
        self
    }

    /// Set response **Status Code**
    pub fn set_status_code(mut self, status: StatusCode) -> Self {
        self.status_code = status;
        self
    }

    /// Set content encoding for serving this file
    #[inline]
    pub fn set_content_encoding(mut self, enc: ContentEncoding) -> Self {
        self.encoding = Some(enc);
        self
    }

    fn etag(&self) -> Option<header::EntityTag> {
        // This etag format is similar to Apache's.
        self.modified.as_ref().map(|mtime| {
            let ino = {
                #[cfg(unix)]
                {
                    self.md.ino()
                }
                #[cfg(not(unix))]
                {
                    0
                }
            };

            let dur = mtime
                .duration_since(UNIX_EPOCH)
                .expect("modification time must be after epoch");
            header::EntityTag::strong(format!(
                "{:x}:{:x}:{:x}:{:x}",
                ino,
                self.md.len(),
                dur.as_secs(),
                dur.subsec_nanos()
            ))
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
fn any_match<S>(etag: Option<&header::EntityTag>, req: &HttpRequest<S>) -> bool {
    match req.get_header::<header::IfMatch>() {
        None | Some(header::IfMatch::Any) => true,
        Some(header::IfMatch::Items(ref items)) => {
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
fn none_match<S>(etag: Option<&header::EntityTag>, req: &HttpRequest<S>) -> bool {
    match req.get_header::<header::IfNoneMatch>() {
        Some(header::IfNoneMatch::Any) => false,
        Some(header::IfNoneMatch::Items(ref items)) => {
            if let Some(some_etag) = etag {
                for item in items {
                    if item.weak_eq(some_etag) {
                        return false;
                    }
                }
            }
            true
        }
        None => true,
    }
}

impl Responder for NamedFile {
    type Item = HttpResponse;
    type Error = io::Error;

    fn respond_to<S>(self, req: &HttpRequest<S>) -> Result<HttpResponse, io::Error> {
        if self.status_code != StatusCode::OK {
            let mut resp = HttpResponse::build(self.status_code);
            resp.if_some(self.path().extension(), |ext, resp| {
                resp.set(header::ContentType(get_mime_type(&ext.to_string_lossy())));
            }).if_some(self.path().file_name(), |file_name, resp| {
                let mime_type = guess_mime_type(self.path());
                let inline_or_attachment = match mime_type.type_() {
                    mime::IMAGE | mime::TEXT | mime::VIDEO => "inline",
                    _ => "attachment",
                };
                resp.header(
                    "Content-Disposition",
                    format!(
                        "{inline_or_attachment}; filename={filename}",
                        inline_or_attachment = inline_or_attachment,
                        filename = file_name.to_string_lossy()
                    ),
                );
            });
            if let Some(current_encoding) = self.encoding {
                resp.content_encoding(current_encoding);
            }
            let reader = ChunkedReadFile {
                size: self.md.len(),
                offset: 0,
                cpu_pool: self.cpu_pool.unwrap_or_else(|| req.cpu_pool().clone()),
                file: Some(self.file),
                fut: None,
                counter: 0,
            };
            return Ok(resp.streaming(reader));
        }

        if self.only_get && *req.method() != Method::GET && *req.method() != Method::HEAD
        {
            return Ok(HttpResponse::MethodNotAllowed()
                .header(header::CONTENT_TYPE, "text/plain")
                .header(header::ALLOW, "GET, HEAD")
                .body("This resource only supports GET and HEAD."));
        }

        let etag = self.etag();
        let last_modified = self.last_modified();

        // check preconditions
        let precondition_failed = if !any_match(etag.as_ref(), req) {
            true
        } else if let (Some(ref m), Some(header::IfUnmodifiedSince(ref since))) =
            (last_modified, req.get_header())
        {
            m > since
        } else {
            false
        };

        // check last modified
        let not_modified = if !none_match(etag.as_ref(), req) {
            true
        } else if let (Some(ref m), Some(header::IfModifiedSince(ref since))) =
            (last_modified, req.get_header())
        {
            m <= since
        } else {
            false
        };

        let mut resp = HttpResponse::build(self.status_code);
        if let Some(current_encoding) = self.encoding {
            resp.content_encoding(current_encoding);
        }

        resp.if_some(self.path().extension(), |ext, resp| {
            resp.set(header::ContentType(get_mime_type(&ext.to_string_lossy())));
        }).if_some(self.path().file_name(), |file_name, resp| {
                let mime_type = guess_mime_type(self.path());
                let inline_or_attachment = match mime_type.type_() {
                    mime::IMAGE | mime::TEXT | mime::VIDEO => "inline",
                    _ => "attachment",
                };
                resp.header(
                    "Content-Disposition",
                    format!(
                        "{inline_or_attachment}; filename={filename}",
                        inline_or_attachment = inline_or_attachment,
                        filename = file_name.to_string_lossy()
                    ),
                );
            })
            .if_some(last_modified, |lm, resp| {
                resp.set(header::LastModified(lm));
            })
            .if_some(etag, |etag, resp| {
                resp.set(header::ETag(etag));
            });

        resp.header(header::ACCEPT_RANGES, "bytes");

        let mut length = self.md.len();
        let mut offset = 0;

        // check for range header
        if let Some(ranges) = req.headers().get(header::RANGE) {
            if let Ok(rangesheader) = ranges.to_str() {
                if let Ok(rangesvec) = HttpRange::parse(rangesheader, length) {
                    length = rangesvec[0].length;
                    offset = rangesvec[0].start;
                    resp.content_encoding(ContentEncoding::Identity);
                    resp.header(
                        header::CONTENT_RANGE,
                        format!(
                            "bytes {}-{}/{}",
                            offset,
                            offset + length - 1,
                            self.md.len()
                        ),
                    );
                } else {
                    resp.header(header::CONTENT_RANGE, format!("bytes */{}", length));
                    return Ok(resp.status(StatusCode::RANGE_NOT_SATISFIABLE).finish());
                };
            } else {
                return Ok(resp.status(StatusCode::BAD_REQUEST).finish());
            };
        };

        resp.header(header::CONTENT_LENGTH, format!("{}", length));

        if precondition_failed {
            return Ok(resp.status(StatusCode::PRECONDITION_FAILED).finish());
        } else if not_modified {
            return Ok(resp.status(StatusCode::NOT_MODIFIED).finish());
        }

        if *req.method() == Method::HEAD {
            Ok(resp.finish())
        } else {
            let reader = ChunkedReadFile {
                offset,
                size: length,
                cpu_pool: self.cpu_pool.unwrap_or_else(|| req.cpu_pool().clone()),
                file: Some(self.file),
                fut: None,
                counter: 0,
            };
            if offset != 0 || length != self.md.len() {
                return Ok(resp.status(StatusCode::PARTIAL_CONTENT).streaming(reader));
            };
            Ok(resp.streaming(reader))
        }
    }
}

/// A helper created from a `std::fs::File` which reads the file
/// chunk-by-chunk on a `CpuPool`.
pub struct ChunkedReadFile {
    size: u64,
    offset: u64,
    cpu_pool: CpuPool,
    file: Option<File>,
    fut: Option<CpuFuture<(File, Bytes), io::Error>>,
    counter: u64,
}

impl Stream for ChunkedReadFile {
    type Item = Bytes;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Bytes>, Error> {
        if self.fut.is_some() {
            return match self.fut.as_mut().unwrap().poll()? {
                Async::Ready((file, bytes)) => {
                    self.fut.take();
                    self.file = Some(file);
                    self.offset += bytes.len() as u64;
                    self.counter += bytes.len() as u64;
                    Ok(Async::Ready(Some(bytes)))
                }
                Async::NotReady => Ok(Async::NotReady),
            };
        }

        let size = self.size;
        let offset = self.offset;
        let counter = self.counter;

        if size == counter {
            Ok(Async::Ready(None))
        } else {
            let mut file = self.file.take().expect("Use after completion");
            self.fut = Some(self.cpu_pool.spawn_fn(move || {
                let max_bytes: usize;
                max_bytes = cmp::min(size.saturating_sub(counter), 65_536) as usize;
                let mut buf = BytesMut::from(Vec::with_capacity(max_bytes));
                file.seek(io::SeekFrom::Start(offset))?;
                let nbytes = file.read(unsafe { buf.bytes_mut() })?;
                if nbytes == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                unsafe { buf.advance_mut(nbytes) };
                Ok((file, buf.freeze()))
            }));
            self.poll()
        }
    }
}

type DirectoryRenderer<S> =
    Fn(&Directory, &HttpRequest<S>) -> Result<HttpResponse, io::Error>;

/// A directory; responds with the generated directory listing.
#[derive(Debug)]
pub struct Directory {
    /// Base directory
    pub base: PathBuf,
    /// Path of subdirectory to generate listing for
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
}

fn directory_listing<S>(
    dir: &Directory, req: &HttpRequest<S>,
) -> Result<HttpResponse, io::Error> {
    let index_of = format!("Index of {}", req.path());
    let mut body = String::new();
    let base = Path::new(req.path());

    for entry in dir.path.read_dir()? {
        if dir.is_visible(&entry) {
            let entry = entry.unwrap();
            let p = match entry.path().strip_prefix(&dir.path) {
                Ok(p) => base.join(p),
                Err(_) => continue,
            };
            // show file url as relative to static path
            let file_url = format!("{}", p.to_string_lossy());

            // if file is a directory, add '/' to the end of the name
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    let _ = write!(
                        body,
                        "<li><a href=\"{}\">{}/</a></li>",
                        file_url,
                        entry.file_name().to_string_lossy()
                    );
                } else {
                    let _ = write!(
                        body,
                        "<li><a href=\"{}\">{}</a></li>",
                        file_url,
                        entry.file_name().to_string_lossy()
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
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html))
}

/// Static files handling
///
/// `StaticFile` handler must be registered with `App::handler()` method,
/// because `StaticFile` handler requires access sub-path information.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{fs, App};
///
/// fn main() {
///     let app = App::new()
///         .handler("/static", fs::StaticFiles::new("."))
///         .finish();
/// }
/// ```
pub struct StaticFiles<S> {
    directory: PathBuf,
    accessible: bool,
    index: Option<String>,
    show_index: bool,
    cpu_pool: CpuPool,
    default: Box<RouteHandler<S>>,
    renderer: Box<DirectoryRenderer<S>>,
    _chunk_size: usize,
    _follow_symlinks: bool,
}

lazy_static! {
    static ref DEFAULT_CPUPOOL: Mutex<CpuPool> = {
        let default = match env::var(ENV_CPU_POOL_VAR) {
            Ok(val) => {
                if let Ok(val) = val.parse() {
                    val
                } else {
                    error!("Can not parse ACTIX_FS_POOL value");
                    20
                }
            }
            Err(_) => 20,
        };
        Mutex::new(CpuPool::new(default))
    };
}

impl<S: 'static> StaticFiles<S> {
    /// Create new `StaticFiles` instance for specified base directory.
    ///
    /// `StaticFile` uses `CpuPool` for blocking filesystem operations.
    /// By default pool with 20 threads is used.
    /// Pool size can be changed by setting ACTIX_FS_POOL environment variable.
    pub fn new<T: Into<PathBuf>>(dir: T) -> StaticFiles<S> {
        // use default CpuPool
        let pool = { DEFAULT_CPUPOOL.lock().unwrap().clone() };

        StaticFiles::with_pool(dir, pool)
    }

    /// Create new `StaticFiles` instance for specified base directory and
    /// `CpuPool`.
    pub fn with_pool<T: Into<PathBuf>>(dir: T, pool: CpuPool) -> StaticFiles<S> {
        let dir = dir.into();

        let (dir, access) = match dir.canonicalize() {
            Ok(dir) => {
                if dir.is_dir() {
                    (dir, true)
                } else {
                    warn!("Is not directory `{:?}`", dir);
                    (dir, false)
                }
            }
            Err(err) => {
                warn!("Static files directory `{:?}` error: {}", dir, err);
                (dir, false)
            }
        };

        StaticFiles {
            directory: dir,
            accessible: access,
            index: None,
            show_index: false,
            cpu_pool: pool,
            default: Box::new(WrapHandler::new(|_| {
                HttpResponse::new(StatusCode::NOT_FOUND)
            })),
            renderer: Box::new(directory_listing),
            _chunk_size: 0,
            _follow_symlinks: false,
        }
    }

    /// Show files listing for directories.
    ///
    /// By default show files listing is disabled.
    pub fn show_files_listing(mut self) -> Self {
        self.show_index = true;
        self
    }

    /// Set custom directory renderer
    pub fn files_listing_renderer<F>(mut self, f: F) -> Self
    where
        for<'r, 's> F: Fn(&'r Directory, &'s HttpRequest<S>)
                -> Result<HttpResponse, io::Error>
            + 'static,
    {
        self.renderer = Box::new(f);
        self
    }

    /// Set index file
    ///
    /// Redirects to specific index file for directory "/" instead of
    /// showing files listing.
    pub fn index_file<T: Into<String>>(mut self, index: T) -> StaticFiles<S> {
        self.index = Some(index.into());
        self
    }

    /// Sets default handler which is used when no matched file could be found.
    pub fn default_handler<H: Handler<S>>(mut self, handler: H) -> StaticFiles<S> {
        self.default = Box::new(WrapHandler::new(handler));
        self
    }
}

impl<S: 'static> Handler<S> for StaticFiles<S> {
    type Result = Result<AsyncResult<HttpResponse>, Error>;

    fn handle(&mut self, req: HttpRequest<S>) -> Self::Result {
        if !self.accessible {
            Ok(self.default.handle(req))
        } else {
            let relpath = match req
                .match_info()
                .get("tail")
                .map(|tail| PathBuf::from_param(tail.trim_left_matches('/')))
            {
                Some(Ok(path)) => path,
                _ => return Ok(self.default.handle(req)),
            };

            // full filepath
            let path = self.directory.join(&relpath).canonicalize()?;

            if path.is_dir() {
                if let Some(ref redir_index) = self.index {
                    // TODO: Don't redirect, just return the index content.
                    // TODO: It'd be nice if there were a good usable URL manipulation
                    // library
                    let mut new_path: String = req.path().to_owned();
                    for el in relpath.iter() {
                        new_path.push_str(&el.to_string_lossy());
                        new_path.push('/');
                    }
                    if !new_path.ends_with('/') {
                        new_path.push('/');
                    }
                    new_path.push_str(redir_index);
                    HttpResponse::Found()
                        .header(header::LOCATION, new_path.as_str())
                        .finish()
                        .respond_to(&req)
                } else if self.show_index {
                    let dir = Directory::new(self.directory.clone(), path);
                    Ok((*self.renderer)(&dir, &req)?.into())
                } else {
                    Ok(self.default.handle(req))
                }
            } else {
                NamedFile::open(path)?
                    .set_cpu_pool(self.cpu_pool.clone())
                    .respond_to(&req)?
                    .respond_to(&req)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::App;
    use http::{header, Method, StatusCode};
    use test::{self, TestRequest};

    #[test]
    fn test_file_extension_to_mime() {
        let m = file_extension_to_mime("jpg");
        assert_eq!(m, mime::IMAGE_JPEG);

        let m = file_extension_to_mime("invalid extension!!");
        assert_eq!(m, mime::APPLICATION_OCTET_STREAM);

        let m = file_extension_to_mime("");
        assert_eq!(m, mime::APPLICATION_OCTET_STREAM);
    }

    #[test]
    fn test_named_file_text() {
        assert!(NamedFile::open("test--").is_err());
        let mut file = NamedFile::open("Cargo.toml")
            .unwrap()
            .set_cpu_pool(CpuPool::new(1));
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let resp = file.respond_to(&HttpRequest::default()).unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-toml"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=Cargo.toml"
        );
    }

    #[test]
    fn test_named_file_image() {
        let mut file = NamedFile::open("tests/test.png")
            .unwrap()
            .set_cpu_pool(CpuPool::new(1));
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let resp = file.respond_to(&HttpRequest::default()).unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=test.png"
        );
    }

    #[test]
    fn test_named_file_binary() {
        let mut file = NamedFile::open("tests/test.binary")
            .unwrap()
            .set_cpu_pool(CpuPool::new(1));
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let resp = file.respond_to(&HttpRequest::default()).unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=test.binary"
        );
    }

    #[test]
    fn test_named_file_status_code_text() {
        let mut file = NamedFile::open("Cargo.toml")
            .unwrap()
            .set_status_code(StatusCode::NOT_FOUND)
            .set_cpu_pool(CpuPool::new(1));
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let resp = file.respond_to(&HttpRequest::default()).unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-toml"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=Cargo.toml"
        );
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_named_file_ranges_status_code() {
        let mut srv = test::TestServer::with_factory(|| {
            App::new().handler("test", StaticFiles::new(".").index_file("Cargo.toml"))
        });

        // Valid range header
        let request = srv
            .get()
            .uri(srv.url("/t%65st/Cargo.toml"))
            .header(header::RANGE, "bytes=10-20")
            .finish()
            .unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);

        // Invalid range header
        let request = srv
            .get()
            .uri(srv.url("/t%65st/Cargo.toml"))
            .header(header::RANGE, "bytes=1-0")
            .finish()
            .unwrap();
        let response = srv.execute(request.send()).unwrap();

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    }

    #[test]
    fn test_named_file_content_range_headers() {
        let mut srv = test::TestServer::with_factory(|| {
            App::new().handler(
                "test",
                StaticFiles::new(".").index_file("tests/test.binary"),
            )
        });

        // Valid range header
        let request = srv
            .get()
            .uri(srv.url("/t%65st/tests/test.binary"))
            .header(header::RANGE, "bytes=10-20")
            .finish()
            .unwrap();

        let response = srv.execute(request.send()).unwrap();

        let contentrange = response
            .headers()
            .get(header::CONTENT_RANGE)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentrange, "bytes 10-20/100");

        // Invalid range header
        let request = srv
            .get()
            .uri(srv.url("/t%65st/tests/test.binary"))
            .header(header::RANGE, "bytes=10-5")
            .finish()
            .unwrap();

        let response = srv.execute(request.send()).unwrap();

        let contentrange = response
            .headers()
            .get(header::CONTENT_RANGE)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentrange, "bytes */100");
    }

    #[test]
    fn test_named_file_content_length_headers() {
        let mut srv = test::TestServer::with_factory(|| {
            App::new().handler(
                "test",
                StaticFiles::new(".").index_file("tests/test.binary"),
            )
        });

        // Valid range header
        let request = srv
            .get()
            .uri(srv.url("/t%65st/tests/test.binary"))
            .header(header::RANGE, "bytes=10-20")
            .finish()
            .unwrap();

        let response = srv.execute(request.send()).unwrap();

        let contentlength = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentlength, "11");

        // Invalid range header
        let request = srv
            .get()
            .uri(srv.url("/t%65st/tests/test.binary"))
            .header(header::RANGE, "bytes=10-8")
            .finish()
            .unwrap();

        let response = srv.execute(request.send()).unwrap();

        let contentlength = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentlength, "0");

        // Without range header
        let request = srv
            .get()
            .uri(srv.url("/t%65st/tests/test.binary"))
            .no_default_headers()
            .finish()
            .unwrap();

        let response = srv.execute(request.send()).unwrap();

        let contentlength = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentlength, "100");

        // chunked
        let request = srv
            .get()
            .uri(srv.url("/t%65st/tests/test.binary"))
            .finish()
            .unwrap();

        let response = srv.execute(request.send()).unwrap();

        let te = response
            .headers()
            .get(header::TRANSFER_ENCODING)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(te, "chunked");
    }

    #[test]
    fn test_named_file_not_allowed() {
        let req = TestRequest::default().method(Method::POST).finish();
        let file = NamedFile::open("Cargo.toml").unwrap();

        let resp = file.only_get().respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_named_file_content_encoding() {
        let req = TestRequest::default().method(Method::GET).finish();
        let file = NamedFile::open("Cargo.toml").unwrap();

        assert!(file.encoding.is_none());
        let resp = file
            .set_content_encoding(ContentEncoding::Identity)
            .respond_to(&req)
            .unwrap();

        assert!(resp.content_encoding().is_some());
        assert_eq!(resp.content_encoding().unwrap().as_str(), "identity");
    }

    #[test]
    fn test_named_file_any_method() {
        let req = TestRequest::default().method(Method::POST).finish();
        let file = NamedFile::open("Cargo.toml").unwrap();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_static_files() {
        let mut st = StaticFiles::new(".").show_files_listing();
        st.accessible = false;
        let resp = st
            .handle(HttpRequest::default())
            .respond_to(&HttpRequest::default())
            .unwrap();
        let resp = resp.as_msg();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        st.accessible = true;
        st.show_index = false;
        let resp = st
            .handle(HttpRequest::default())
            .respond_to(&HttpRequest::default())
            .unwrap();
        let resp = resp.as_msg();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "");

        st.show_index = true;
        let resp = st.handle(req).respond_to(&HttpRequest::default()).unwrap();
        let resp = resp.as_msg();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert!(resp.body().is_binary());
        assert!(format!("{:?}", resp.body()).contains("README.md"));
    }

    #[test]
    fn test_redirect_to_index() {
        let mut st = StaticFiles::new(".").index_file("index.html");
        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "tests");

        let resp = st.handle(req).respond_to(&HttpRequest::default()).unwrap();
        let resp = resp.as_msg();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/tests/index.html"
        );

        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "tests/");

        let resp = st.handle(req).respond_to(&HttpRequest::default()).unwrap();
        let resp = resp.as_msg();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/tests/index.html"
        );
    }

    #[test]
    fn test_redirect_to_index_nested() {
        let mut st = StaticFiles::new(".").index_file("Cargo.toml");
        let mut req = HttpRequest::default();
        req.match_info_mut().add("tail", "tools/wsload");

        let resp = st.handle(req).respond_to(&HttpRequest::default()).unwrap();
        let resp = resp.as_msg();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/tools/wsload/Cargo.toml"
        );
    }

    #[test]
    fn integration_redirect_to_index_with_prefix() {
        let mut srv = test::TestServer::with_factory(|| {
            App::new()
                .prefix("public")
                .handler("/", StaticFiles::new(".").index_file("Cargo.toml"))
        });

        let request = srv.get().uri(srv.url("/public")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let loc = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/public/Cargo.toml");

        let request = srv.get().uri(srv.url("/public/")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let loc = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/public/Cargo.toml");
    }

    #[test]
    fn integration_redirect_to_index() {
        let mut srv = test::TestServer::with_factory(|| {
            App::new().handler("test", StaticFiles::new(".").index_file("Cargo.toml"))
        });

        let request = srv.get().uri(srv.url("/test")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let loc = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/test/Cargo.toml");

        let request = srv.get().uri(srv.url("/test/")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let loc = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/test/Cargo.toml");
    }

    #[test]
    fn integration_percent_encoded() {
        let mut srv = test::TestServer::with_factory(|| {
            App::new().handler("test", StaticFiles::new(".").index_file("Cargo.toml"))
        });

        let request = srv
            .get()
            .uri(srv.url("/test/%43argo.toml"))
            .finish()
            .unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

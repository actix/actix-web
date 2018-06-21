//! Static files support
use std::fmt::Write;
use std::fs::{DirEntry, File, Metadata};
use std::io::{Read, Seek};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{cmp, io};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use bytes::Bytes;
use futures::{Async, Future, Poll, Stream};
use futures_cpupool::{CpuFuture, CpuPool};
use mime;
use mime_guess::{get_mime_type, guess_mime_type};

use error::Error;
use handler::{AsyncResult, Handler, Responder, RouteHandler, WrapHandler};
use header;
use http::{ContentEncoding, Method, StatusCode};
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use param::FromParam;
use server::settings::DEFAULT_CPUPOOL;

/// Return the MIME type associated with a filename extension (case-insensitive).
/// If `ext` is empty or no associated type for the extension was found, returns
/// the type `application/octet-stream`.
#[inline]
pub fn file_extension_to_mime(ext: &str) -> mime::Mime {
    get_mime_type(ext)
}

/// A file with an associated name.
#[derive(Debug)]
pub struct NamedFile {
    path: PathBuf,
    file: File,
    content_type: mime::Mime,
    content_disposition: header::ContentDisposition,
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
        use header::{ContentDisposition, DispositionParam, DispositionType};
        let path = path.as_ref().to_path_buf();

        // Get the name of the file and use it to construct default Content-Type
        // and Content-Disposition values
        let (content_type, content_disposition) = {
            let filename = match path.file_name() {
                Some(name) => name.to_string_lossy(),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Provided path has no filename",
                    ))
                }
            };

            let ct = guess_mime_type(&path);
            let disposition_type = match ct.type_() {
                mime::IMAGE | mime::TEXT | mime::VIDEO => DispositionType::Inline,
                _ => DispositionType::Attachment,
            };
            let cd = ContentDisposition {
                disposition: disposition_type,
                parameters: vec![DispositionParam::Filename(
                    header::Charset::Ext("UTF-8".to_owned()),
                    None,
                    filename.as_bytes().to_vec(),
                )],
            };
            (ct, cd)
        };

        let file = File::open(&path)?;
        let md = file.metadata()?;
        let modified = md.modified().ok();
        let cpu_pool = None;
        let encoding = None;
        Ok(NamedFile {
            path,
            file,
            content_type,
            content_disposition,
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

    /// Set the MIME Content-Type for serving this file. By default
    /// the Content-Type is inferred from the filename extension.
    #[inline]
    pub fn set_content_type(mut self, mime_type: mime::Mime) -> Self {
        self.content_type = mime_type;
        self
    }

    /// Set the Content-Disposition for serving this file. This allows
    /// changing the inline/attachment disposition as well as the filename
    /// sent to the peer. By default the disposition is `inline` for text,
    /// image, and video content types, and `attachment` otherwise, and
    /// the filename is taken from the path provided in the `open` method
    /// after converting it to UTF-8 using
    /// [to_string_lossy](https://doc.rust-lang.org/std/ffi/struct.OsStr.html#method.to_string_lossy).
    #[inline]
    pub fn set_content_disposition(mut self, cd: header::ContentDisposition) -> Self {
        self.content_disposition = cd;
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
            resp.set(header::ContentType(self.content_type.clone()))
                .header(
                    header::CONTENT_DISPOSITION,
                    self.content_disposition.to_string(),
                );

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
        resp.set(header::ContentType(self.content_type.clone()))
            .header(
                header::CONTENT_DISPOSITION,
                self.content_disposition.to_string(),
            );

        if let Some(current_encoding) = self.encoding {
            resp.content_encoding(current_encoding);
        }

        resp.if_some(last_modified, |lm, resp| {
            resp.set(header::LastModified(lm));
        }).if_some(etag, |etag, resp| {
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
                let mut buf = Vec::with_capacity(max_bytes);
                file.seek(io::SeekFrom::Start(offset))?;
                let nbytes = file.read(buf.as_mut_slice())?;
                if nbytes == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                Ok((file, Bytes::from(buf)))
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

impl<S: 'static> StaticFiles<S> {
    /// Create new `StaticFiles` instance for specified base directory.
    ///
    /// `StaticFile` uses `CpuPool` for blocking filesystem operations.
    /// By default pool with 20 threads is used.
    /// Pool size can be changed by setting ACTIX_CPU_POOL environment variable.
    pub fn new<T: Into<PathBuf>>(dir: T) -> StaticFiles<S> {
        // use default CpuPool
        let pool = { DEFAULT_CPUPOOL.lock().clone() };

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

    fn handle(&self, req: HttpRequest<S>) -> Self::Result {
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

/// HTTP Range header representation.
#[derive(Debug, Clone, Copy)]
struct HttpRange {
    pub start: u64,
    pub length: u64,
}

static PREFIX: &'static str = "bytes=";
const PREFIX_LEN: usize = 6;

impl HttpRange {
    /// Parses Range HTTP header string as per RFC 2616.
    ///
    /// `header` is HTTP Range header (e.g. `bytes=bytes=0-9`).
    /// `size` is full size of response (file).
    fn parse(header: &str, size: u64) -> Result<Vec<HttpRange>, ()> {
        if header.is_empty() {
            return Ok(Vec::new());
        }
        if !header.starts_with(PREFIX) {
            return Err(());
        }

        let size_sig = size as i64;
        let mut no_overlap = false;

        let all_ranges: Vec<Option<HttpRange>> = header[PREFIX_LEN..]
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .map(|ra| {
                let mut start_end_iter = ra.split('-');

                let start_str = start_end_iter.next().ok_or(())?.trim();
                let end_str = start_end_iter.next().ok_or(())?.trim();

                if start_str.is_empty() {
                    // If no start is specified, end specifies the
                    // range start relative to the end of the file.
                    let mut length: i64 = try!(end_str.parse().map_err(|_| ()));

                    if length > size_sig {
                        length = size_sig;
                    }

                    Ok(Some(HttpRange {
                        start: (size_sig - length) as u64,
                        length: length as u64,
                    }))
                } else {
                    let start: i64 = start_str.parse().map_err(|_| ())?;

                    if start < 0 {
                        return Err(());
                    }
                    if start >= size_sig {
                        no_overlap = true;
                        return Ok(None);
                    }

                    let length = if end_str.is_empty() {
                        // If no end is specified, range extends to end of the file.
                        size_sig - start
                    } else {
                        let mut end: i64 = end_str.parse().map_err(|_| ())?;

                        if start > end {
                            return Err(());
                        }

                        if end >= size_sig {
                            end = size_sig - 1;
                        }

                        end - start + 1
                    };

                    Ok(Some(HttpRange {
                        start: start as u64,
                        length: length as u64,
                    }))
                }
            })
            .collect::<Result<_, _>>()?;

        let ranges: Vec<HttpRange> = all_ranges.into_iter().filter_map(|x| x).collect();

        if no_overlap && ranges.is_empty() {
            return Err(());
        }

        Ok(ranges)
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
            "inline; filename=\"Cargo.toml\""
        );
    }

    #[test]
    fn test_named_file_set_content_type() {
        let mut file = NamedFile::open("Cargo.toml")
            .unwrap()
            .set_content_type(mime::TEXT_XML)
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
            "text/xml"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"Cargo.toml\""
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
            "inline; filename=\"test.png\""
        );
    }

    #[test]
    fn test_named_file_image_attachment() {
        use header::{ContentDisposition, DispositionParam, DispositionType};
        let cd = ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![DispositionParam::Filename(
                header::Charset::Ext("UTF-8".to_owned()),
                None,
                "test.png".as_bytes().to_vec(),
            )],
        };
        let mut file = NamedFile::open("tests/test.png")
            .unwrap()
            .set_content_disposition(cd)
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
            "attachment; filename=\"test.png\""
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
            "attachment; filename=\"test.binary\""
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
            "inline; filename=\"Cargo.toml\""
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
        req.match_info_mut().add_static("tail", "");

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
        req.match_info_mut().add_static("tail", "tests");

        let resp = st.handle(req).respond_to(&HttpRequest::default()).unwrap();
        let resp = resp.as_msg();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/tests/index.html"
        );

        let mut req = HttpRequest::default();
        req.match_info_mut().add_static("tail", "tests/");

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
        let mut st = StaticFiles::new(".").index_file("mod.rs");
        let mut req = HttpRequest::default();
        req.match_info_mut().add_static("tail", "src/client");

        let resp = st.handle(req).respond_to(&HttpRequest::default()).unwrap();
        let resp = resp.as_msg();
        assert_eq!(resp.status(), StatusCode::FOUND);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/src/client/mod.rs"
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

    struct T(&'static str, u64, Vec<HttpRange>);

    #[test]
    fn test_parse() {
        let tests = vec![
            T("", 0, vec![]),
            T("", 1000, vec![]),
            T("foo", 0, vec![]),
            T("bytes=", 0, vec![]),
            T("bytes=7", 10, vec![]),
            T("bytes= 7 ", 10, vec![]),
            T("bytes=1-", 0, vec![]),
            T("bytes=5-4", 10, vec![]),
            T("bytes=0-2,5-4", 10, vec![]),
            T("bytes=2-5,4-3", 10, vec![]),
            T("bytes=--5,4--3", 10, vec![]),
            T("bytes=A-", 10, vec![]),
            T("bytes=A- ", 10, vec![]),
            T("bytes=A-Z", 10, vec![]),
            T("bytes= -Z", 10, vec![]),
            T("bytes=5-Z", 10, vec![]),
            T("bytes=Ran-dom, garbage", 10, vec![]),
            T("bytes=0x01-0x02", 10, vec![]),
            T("bytes=         ", 10, vec![]),
            T("bytes= , , ,   ", 10, vec![]),
            T(
                "bytes=0-9",
                10,
                vec![HttpRange {
                    start: 0,
                    length: 10,
                }],
            ),
            T(
                "bytes=0-",
                10,
                vec![HttpRange {
                    start: 0,
                    length: 10,
                }],
            ),
            T(
                "bytes=5-",
                10,
                vec![HttpRange {
                    start: 5,
                    length: 5,
                }],
            ),
            T(
                "bytes=0-20",
                10,
                vec![HttpRange {
                    start: 0,
                    length: 10,
                }],
            ),
            T(
                "bytes=15-,0-5",
                10,
                vec![HttpRange {
                    start: 0,
                    length: 6,
                }],
            ),
            T(
                "bytes=1-2,5-",
                10,
                vec![
                    HttpRange {
                        start: 1,
                        length: 2,
                    },
                    HttpRange {
                        start: 5,
                        length: 5,
                    },
                ],
            ),
            T(
                "bytes=-2 , 7-",
                11,
                vec![
                    HttpRange {
                        start: 9,
                        length: 2,
                    },
                    HttpRange {
                        start: 7,
                        length: 4,
                    },
                ],
            ),
            T(
                "bytes=0-0 ,2-2, 7-",
                11,
                vec![
                    HttpRange {
                        start: 0,
                        length: 1,
                    },
                    HttpRange {
                        start: 2,
                        length: 1,
                    },
                    HttpRange {
                        start: 7,
                        length: 4,
                    },
                ],
            ),
            T(
                "bytes=-5",
                10,
                vec![HttpRange {
                    start: 5,
                    length: 5,
                }],
            ),
            T(
                "bytes=-15",
                10,
                vec![HttpRange {
                    start: 0,
                    length: 10,
                }],
            ),
            T(
                "bytes=0-499",
                10000,
                vec![HttpRange {
                    start: 0,
                    length: 500,
                }],
            ),
            T(
                "bytes=500-999",
                10000,
                vec![HttpRange {
                    start: 500,
                    length: 500,
                }],
            ),
            T(
                "bytes=-500",
                10000,
                vec![HttpRange {
                    start: 9500,
                    length: 500,
                }],
            ),
            T(
                "bytes=9500-",
                10000,
                vec![HttpRange {
                    start: 9500,
                    length: 500,
                }],
            ),
            T(
                "bytes=0-0,-1",
                10000,
                vec![
                    HttpRange {
                        start: 0,
                        length: 1,
                    },
                    HttpRange {
                        start: 9999,
                        length: 1,
                    },
                ],
            ),
            T(
                "bytes=500-600,601-999",
                10000,
                vec![
                    HttpRange {
                        start: 500,
                        length: 101,
                    },
                    HttpRange {
                        start: 601,
                        length: 399,
                    },
                ],
            ),
            T(
                "bytes=500-700,601-999",
                10000,
                vec![
                    HttpRange {
                        start: 500,
                        length: 201,
                    },
                    HttpRange {
                        start: 601,
                        length: 399,
                    },
                ],
            ),
            // Match Apache laxity:
            T(
                "bytes=   1 -2   ,  4- 5, 7 - 8 , ,,",
                11,
                vec![
                    HttpRange {
                        start: 1,
                        length: 2,
                    },
                    HttpRange {
                        start: 4,
                        length: 2,
                    },
                    HttpRange {
                        start: 7,
                        length: 2,
                    },
                ],
            ),
        ];

        for t in tests {
            let header = t.0;
            let size = t.1;
            let expected = t.2;

            let res = HttpRange::parse(header, size);

            if res.is_err() {
                if expected.is_empty() {
                    continue;
                } else {
                    assert!(
                        false,
                        "parse({}, {}) returned error {:?}",
                        header,
                        size,
                        res.unwrap_err()
                    );
                }
            }

            let got = res.unwrap();

            if got.len() != expected.len() {
                assert!(
                    false,
                    "len(parseRange({}, {})) = {}, want {}",
                    header,
                    size,
                    got.len(),
                    expected.len()
                );
                continue;
            }

            for i in 0..expected.len() {
                if got[i].start != expected[i].start {
                    assert!(
                        false,
                        "parseRange({}, {})[{}].start = {}, want {}",
                        header, size, i, got[i].start, expected[i].start
                    )
                }
                if got[i].length != expected[i].length {
                    assert!(
                        false,
                        "parseRange({}, {})[{}].length = {}, want {}",
                        header, size, i, got[i].length, expected[i].length
                    )
                }
            }
        }
    }
}

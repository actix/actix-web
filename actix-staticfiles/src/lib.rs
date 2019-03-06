//! Static files support
use std::cell::RefCell;
use std::fmt::Write;
use std::fs::{DirEntry, File};
use std::io::{Read, Seek};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{cmp, io};

use bytes::Bytes;
use futures::{Async, Future, Poll, Stream};
use mime;
use mime_guess::get_mime_type;
use percent_encoding::{utf8_percent_encode, DEFAULT_ENCODE_SET};
use v_htmlescape::escape as escape_html_entity;

use actix_http::error::{Error, ErrorInternalServerError};
use actix_service::{boxed::BoxedNewService, NewService, Service};
use actix_web::dev::{self, AppConfig, HttpServiceFactory, ResourceDef, Url};
use actix_web::{
    blocking, FromRequest, HttpRequest, HttpResponse, Responder, ServiceFromRequest,
    ServiceRequest, ServiceResponse,
};
use futures::future::{ok, FutureResult};

mod config;
mod error;
mod named;
mod range;

use self::error::{StaticFilesError, UriSegmentError};
pub use crate::config::{DefaultConfig, StaticFileConfig};
pub use crate::named::NamedFile;
pub use crate::range::HttpRange;

type HttpNewService<P> = BoxedNewService<(), ServiceRequest<P>, ServiceResponse, (), ()>;

/// Return the MIME type associated with a filename extension (case-insensitive).
/// If `ext` is empty or no associated type for the extension was found, returns
/// the type `application/octet-stream`.
#[inline]
pub fn file_extension_to_mime(ext: &str) -> mime::Mime {
    get_mime_type(ext)
}

#[doc(hidden)]
/// A helper created from a `std::fs::File` which reads the file
/// chunk-by-chunk on a `ThreadPool`.
pub struct ChunkedReadFile {
    size: u64,
    offset: u64,
    file: Option<File>,
    fut: Option<blocking::CpuFuture<(File, Bytes), io::Error>>,
    counter: u64,
}

fn handle_error(err: blocking::BlockingError<io::Error>) -> Error {
    match err {
        blocking::BlockingError::Error(err) => err.into(),
        blocking::BlockingError::Canceled => {
            ErrorInternalServerError("Unexpected error").into()
        }
    }
}

impl Stream for ChunkedReadFile {
    type Item = Bytes;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Bytes>, Error> {
        if self.fut.is_some() {
            return match self.fut.as_mut().unwrap().poll().map_err(handle_error)? {
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
            self.fut = Some(blocking::run(move || {
                let max_bytes: usize;
                max_bytes = cmp::min(size.saturating_sub(counter), 65_536) as usize;
                let mut buf = Vec::with_capacity(max_bytes);
                file.seek(io::SeekFrom::Start(offset))?;
                let nbytes =
                    file.by_ref().take(max_bytes as u64).read_to_end(&mut buf)?;
                if nbytes == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                Ok((file, Bytes::from(buf)))
            }));
            self.poll()
        }
    }
}

type DirectoryRenderer =
    Fn(&Directory, &HttpRequest) -> Result<ServiceResponse, io::Error>;

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

// show file url as relative to static path
macro_rules! encode_file_url {
    ($path:ident) => {
        utf8_percent_encode(&$path.to_string_lossy(), DEFAULT_ENCODE_SET)
    };
}

// " -- &quot;  & -- &amp;  ' -- &#x27;  < -- &lt;  > -- &gt;  / -- &#x2f;
macro_rules! encode_file_name {
    ($entry:ident) => {
        escape_html_entity(&$entry.file_name().to_string_lossy())
    };
}

fn directory_listing(
    dir: &Directory,
    req: &HttpRequest,
) -> Result<ServiceResponse, io::Error> {
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

/// Static files handling
///
/// `StaticFile` handler must be registered with `App::service()` method.
///
/// ```rust
/// use actix_web::App;
/// use actix_staticfiles as fs;
///
/// fn main() {
///     let app = App::new()
///         .service(fs::StaticFiles::new("/static", "."));
/// }
/// ```
pub struct StaticFiles<S, C = DefaultConfig> {
    path: String,
    directory: PathBuf,
    index: Option<String>,
    show_index: bool,
    default: Rc<RefCell<Option<Rc<HttpNewService<S>>>>>,
    renderer: Rc<DirectoryRenderer>,
    _chunk_size: usize,
    _follow_symlinks: bool,
    _cd_map: PhantomData<C>,
}

impl<S: 'static> StaticFiles<S> {
    /// Create new `StaticFiles` instance for specified base directory.
    ///
    /// `StaticFile` uses `ThreadPool` for blocking filesystem operations.
    /// By default pool with 5x threads of available cpus is used.
    /// Pool size can be changed by setting ACTIX_CPU_POOL environment variable.
    pub fn new<T: Into<PathBuf>>(path: &str, dir: T) -> StaticFiles<S> {
        Self::with_config(path, dir, DefaultConfig)
    }
}

impl<S: 'static, C: StaticFileConfig> StaticFiles<S, C> {
    /// Create new `StaticFiles` instance for specified base directory.
    ///
    /// Identical with `new` but allows to specify configiration to use.
    pub fn with_config<T: Into<PathBuf>>(path: &str, dir: T, _: C) -> StaticFiles<S, C> {
        let dir = dir.into().canonicalize().unwrap_or_else(|_| PathBuf::new());
        if !dir.is_dir() {
            log::error!("Specified path is not a directory");
        }

        StaticFiles {
            path: path.to_string(),
            directory: dir,
            index: None,
            show_index: false,
            default: Rc::new(RefCell::new(None)),
            renderer: Rc::new(directory_listing),
            _chunk_size: 0,
            _follow_symlinks: false,
            _cd_map: PhantomData,
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
        for<'r, 's> F:
            Fn(&'r Directory, &'s HttpRequest) -> Result<ServiceResponse, io::Error>
                + 'static,
    {
        self.renderer = Rc::new(f);
        self
    }

    /// Set index file
    ///
    /// Shows specific index file for directory "/" instead of
    /// showing files listing.
    pub fn index_file<T: Into<String>>(mut self, index: T) -> StaticFiles<S, C> {
        self.index = Some(index.into());
        self
    }
}

impl<P, C> HttpServiceFactory<P> for StaticFiles<P, C>
where
    P: 'static,
    C: StaticFileConfig + 'static,
{
    fn register(self, config: &mut AppConfig<P>) {
        if self.default.borrow().is_none() {
            *self.default.borrow_mut() = Some(config.default_service());
        }
        let rdef = if config.is_root() {
            ResourceDef::root_prefix(&self.path)
        } else {
            ResourceDef::prefix(&self.path)
        };
        config.register_service(rdef, None, self)
    }
}

impl<P, C: StaticFileConfig + 'static> NewService<ServiceRequest<P>>
    for StaticFiles<P, C>
{
    type Response = ServiceResponse;
    type Error = ();
    type Service = StaticFilesService<P, C>;
    type InitError = ();
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(StaticFilesService {
            directory: self.directory.clone(),
            index: self.index.clone(),
            show_index: self.show_index,
            default: self.default.clone(),
            renderer: self.renderer.clone(),
            _chunk_size: self._chunk_size,
            _follow_symlinks: self._follow_symlinks,
            _cd_map: self._cd_map,
        })
    }
}

pub struct StaticFilesService<S, C = DefaultConfig> {
    directory: PathBuf,
    index: Option<String>,
    show_index: bool,
    default: Rc<RefCell<Option<Rc<HttpNewService<S>>>>>,
    renderer: Rc<DirectoryRenderer>,
    _chunk_size: usize,
    _follow_symlinks: bool,
    _cd_map: PhantomData<C>,
}

impl<P, C: StaticFileConfig> Service<ServiceRequest<P>> for StaticFilesService<P, C> {
    type Response = ServiceResponse;
    type Error = ();
    type Future = FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: ServiceRequest<P>) -> Self::Future {
        let (req, _) = req.into_parts();

        let real_path = match PathBufWrp::get_pathbuf(req.match_info()) {
            Ok(item) => item,
            Err(e) => return ok(ServiceResponse::from_err(e, req.clone())),
        };

        // full filepath
        let path = match self.directory.join(&real_path.0).canonicalize() {
            Ok(path) => path,
            Err(e) => return ok(ServiceResponse::from_err(e, req.clone())),
        };

        if path.is_dir() {
            if let Some(ref redir_index) = self.index {
                let path = path.join(redir_index);

                match NamedFile::open_with_config(path, C::default()) {
                    Ok(named_file) => match named_file.respond_to(&req) {
                        Ok(item) => ok(ServiceResponse::new(req.clone(), item)),
                        Err(e) => ok(ServiceResponse::from_err(e, req.clone())),
                    },
                    Err(e) => ok(ServiceResponse::from_err(e, req.clone())),
                }
            } else if self.show_index {
                let dir = Directory::new(self.directory.clone(), path);
                let x = (self.renderer)(&dir, &req);
                match x {
                    Ok(resp) => ok(resp),
                    Err(e) => ok(ServiceResponse::from_err(e, req.clone())),
                }
            } else {
                ok(ServiceResponse::from_err(
                    StaticFilesError::IsDirectory,
                    req.clone(),
                ))
            }
        } else {
            match NamedFile::open_with_config(path, C::default()) {
                Ok(named_file) => match named_file.respond_to(&req) {
                    Ok(item) => ok(ServiceResponse::new(req.clone(), item)),
                    Err(e) => ok(ServiceResponse::from_err(e, req.clone())),
                },
                Err(e) => ok(ServiceResponse::from_err(e, req.clone())),
            }
        }
    }
}

struct PathBufWrp(PathBuf);

impl PathBufWrp {
    fn get_pathbuf(path: &dev::Path<Url>) -> Result<Self, UriSegmentError> {
        let path_str = path.path();
        let mut buf = PathBuf::new();
        for segment in path_str.split('/') {
            if segment == ".." {
                buf.pop();
            } else if segment.starts_with('.') {
                return Err(UriSegmentError::BadStart('.'));
            } else if segment.starts_with('*') {
                return Err(UriSegmentError::BadStart('*'));
            } else if segment.ends_with(':') {
                return Err(UriSegmentError::BadEnd(':'));
            } else if segment.ends_with('>') {
                return Err(UriSegmentError::BadEnd('>'));
            } else if segment.ends_with('<') {
                return Err(UriSegmentError::BadEnd('<'));
            } else if segment.is_empty() {
                continue;
            } else if cfg!(windows) && segment.contains('\\') {
                return Err(UriSegmentError::BadChar('\\'));
            } else {
                buf.push(segment)
            }
        }

        Ok(PathBufWrp(buf))
    }
}

impl<P> FromRequest<P> for PathBufWrp {
    type Error = UriSegmentError;
    type Future = Result<Self, Self::Error>;
    type Config = ();

    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        PathBufWrp::get_pathbuf(req.match_info())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::ops::Add;
    use std::time::{Duration, SystemTime};

    use bytes::BytesMut;

    use super::*;
    use actix_web::http::{header, header::DispositionType, Method, StatusCode};
    use actix_web::test::{self, TestRequest};
    use actix_web::App;

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
    fn test_if_modified_since_without_if_none_match() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since =
            header::HttpDate::from(SystemTime::now().add(Duration::from_secs(60)));

        let req = TestRequest::default()
            .header(header::IF_MODIFIED_SINCE, since)
            .to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn test_if_modified_since_with_if_none_match() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let since =
            header::HttpDate::from(SystemTime::now().add(Duration::from_secs(60)));

        let req = TestRequest::default()
            .header(header::IF_NONE_MATCH, "miss_etag")
            .header(header::IF_MODIFIED_SINCE, since)
            .to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_ne!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn test_named_file_text() {
        assert!(NamedFile::open("test--").is_err());
        let mut file = NamedFile::open("Cargo.toml").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
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
            .set_content_type(mime::TEXT_XML);
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
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
        let mut file = NamedFile::open("tests/test.png").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
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
            parameters: vec![DispositionParam::Filename(String::from("test.png"))],
        };
        let mut file = NamedFile::open("tests/test.png")
            .unwrap()
            .set_content_disposition(cd);
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=\"test.png\""
        );
    }

    #[derive(Default)]
    pub struct AllAttachmentConfig;
    impl StaticFileConfig for AllAttachmentConfig {
        fn content_disposition_map(_typ: mime::Name) -> DispositionType {
            DispositionType::Attachment
        }
    }

    #[derive(Default)]
    pub struct AllInlineConfig;
    impl StaticFileConfig for AllInlineConfig {
        fn content_disposition_map(_typ: mime::Name) -> DispositionType {
            DispositionType::Inline
        }
    }

    #[test]
    fn test_named_file_image_attachment_and_custom_config() {
        let file =
            NamedFile::open_with_config("tests/test.png", AllAttachmentConfig).unwrap();

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "attachment; filename=\"test.png\""
        );

        let file =
            NamedFile::open_with_config("tests/test.png", AllInlineConfig).unwrap();

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
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
    fn test_named_file_binary() {
        let mut file = NamedFile::open("tests/test.binary").unwrap();
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
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
            .set_status_code(StatusCode::NOT_FOUND);
        {
            file.file();
            let _f: &File = &file;
        }
        {
            let _f: &mut File = &mut file;
        }

        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
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
        let mut srv = test::init_service(
            App::new().service(StaticFiles::new("/test", ".").index_file("Cargo.toml")),
        );

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .header(header::RANGE, "bytes=10-20")
            .to_request();
        let response = test::call_success(&mut srv, request);
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);

        // Invalid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .header(header::RANGE, "bytes=1-0")
            .to_request();
        let response = test::call_success(&mut srv, request);

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    }

    #[test]
    fn test_named_file_content_range_headers() {
        let mut srv = test::init_service(
            App::new()
                .service(StaticFiles::new("/test", ".").index_file("tests/test.binary")),
        );

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .header(header::RANGE, "bytes=10-20")
            .to_request();

        let response = test::call_success(&mut srv, request);
        let contentrange = response
            .headers()
            .get(header::CONTENT_RANGE)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentrange, "bytes 10-20/100");

        // Invalid range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .header(header::RANGE, "bytes=10-5")
            .to_request();
        let response = test::call_success(&mut srv, request);

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
        let mut srv = test::init_service(
            App::new()
                .service(StaticFiles::new("test", ".").index_file("tests/test.binary")),
        );

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .header(header::RANGE, "bytes=10-20")
            .to_request();
        let response = test::call_success(&mut srv, request);

        let contentlength = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentlength, "11");

        // Invalid range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .header(header::RANGE, "bytes=10-8")
            .to_request();
        let response = test::call_success(&mut srv, request);
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        // Without range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            // .no_default_headers()
            .to_request();
        let response = test::call_success(&mut srv, request);

        let contentlength = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(contentlength, "100");

        // chunked
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .to_request();
        let mut response = test::call_success(&mut srv, request);

        // with enabled compression
        // {
        //     let te = response
        //         .headers()
        //         .get(header::TRANSFER_ENCODING)
        //         .unwrap()
        //         .to_str()
        //         .unwrap();
        //     assert_eq!(te, "chunked");
        // }

        let bytes =
            test::block_on(response.take_body().fold(BytesMut::new(), |mut b, c| {
                b.extend(c);
                Ok::<_, Error>(b)
            }))
            .unwrap();
        let data = Bytes::from(fs::read("tests/test.binary").unwrap());
        assert_eq!(bytes.freeze(), data);
    }

    #[test]
    fn test_static_files_with_spaces() {
        let mut srv = test::init_service(
            App::new().service(StaticFiles::new("/", ".").index_file("Cargo.toml")),
        );
        let request = TestRequest::get()
            .uri("/tests/test%20space.binary")
            .to_request();
        let mut response = test::call_success(&mut srv, request);
        assert_eq!(response.status(), StatusCode::OK);

        let bytes =
            test::block_on(response.take_body().fold(BytesMut::new(), |mut b, c| {
                b.extend(c);
                Ok::<_, Error>(b)
            }))
            .unwrap();

        let data = Bytes::from(fs::read("tests/test space.binary").unwrap());
        assert_eq!(bytes.freeze(), data);
    }

    #[derive(Default)]
    pub struct OnlyMethodHeadConfig;
    impl StaticFileConfig for OnlyMethodHeadConfig {
        fn is_method_allowed(method: &Method) -> bool {
            match *method {
                Method::HEAD => true,
                _ => false,
            }
        }
    }

    #[test]
    fn test_named_file_not_allowed() {
        let file =
            NamedFile::open_with_config("Cargo.toml", OnlyMethodHeadConfig).unwrap();
        let req = TestRequest::default()
            .method(Method::POST)
            .to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let file =
            NamedFile::open_with_config("Cargo.toml", OnlyMethodHeadConfig).unwrap();
        let req = TestRequest::default().method(Method::PUT).to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let file =
            NamedFile::open_with_config("Cargo.toml", OnlyMethodHeadConfig).unwrap();
        let req = TestRequest::default().method(Method::GET).to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    //     #[test]
    //     fn test_named_file_content_encoding() {
    //         let req = TestRequest::default().method(Method::GET).finish();
    //         let file = NamedFile::open("Cargo.toml").unwrap();

    //         assert!(file.encoding.is_none());
    //         let resp = file
    //             .set_content_encoding(ContentEncoding::Identity)
    //             .respond_to(&req)
    //             .unwrap();

    //         assert!(resp.content_encoding().is_some());
    //         assert_eq!(resp.content_encoding().unwrap().as_str(), "identity");
    //     }

    #[test]
    fn test_named_file_any_method() {
        let req = TestRequest::default()
            .method(Method::POST)
            .to_http_request();
        let file = NamedFile::open("Cargo.toml").unwrap();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_static_files() {
        let mut srv = test::init_service(
            App::new().service(StaticFiles::new("/", ".").show_files_listing()),
        );
        let req = TestRequest::with_uri("/missing").to_request();

        let resp = test::call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = test::init_service(App::new().service(StaticFiles::new("/", ".")));

        let req = TestRequest::default().to_request();
        let resp = test::call_success(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = test::init_service(
            App::new().service(StaticFiles::new("/", ".").show_files_listing()),
        );
        let req = TestRequest::with_uri("/tests").to_request();
        let mut resp = test::call_success(&mut srv, req);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );

        let bytes =
            test::block_on(resp.take_body().fold(BytesMut::new(), |mut b, c| {
                b.extend(c);
                Ok::<_, Error>(b)
            }))
            .unwrap();
        assert!(format!("{:?}", bytes).contains("/tests/test.png"));
    }

    #[test]
    fn test_static_files_bad_directory() {
        let _st: StaticFiles<()> = StaticFiles::new("/", "missing");
        let _st: StaticFiles<()> = StaticFiles::new("/", "Cargo.toml");
    }

    //     #[test]
    //     fn test_default_handler_file_missing() {
    //         let st = StaticFiles::new(".")
    //             .default_handler(|_: &_| "default content");
    //         let req = TestRequest::with_uri("/missing")
    //             .param("tail", "missing")
    //             .finish();

    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::OK);
    //         assert_eq!(
    //             resp.body(),
    //             &Body::Binary(Binary::Slice(b"default content"))
    //         );
    //     }

    //     #[test]
    //     fn test_serve_index() {
    //         let st = StaticFiles::new(".").index_file("test.binary");
    //         let req = TestRequest::default().uri("/tests").finish();

    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::OK);
    //         assert_eq!(
    //             resp.headers()
    //                 .get(header::CONTENT_TYPE)
    //                 .expect("content type"),
    //             "application/octet-stream"
    //         );
    //         assert_eq!(
    //             resp.headers()
    //                 .get(header::CONTENT_DISPOSITION)
    //                 .expect("content disposition"),
    //             "attachment; filename=\"test.binary\""
    //         );

    //         let req = TestRequest::default().uri("/tests/").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::OK);
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_TYPE).unwrap(),
    //             "application/octet-stream"
    //         );
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
    //             "attachment; filename=\"test.binary\""
    //         );

    //         // nonexistent index file
    //         let req = TestRequest::default().uri("/tests/unknown").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    //         let req = TestRequest::default().uri("/tests/unknown/").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    //     }

    //     #[test]
    //     fn test_serve_index_nested() {
    //         let st = StaticFiles::new(".").index_file("mod.rs");
    //         let req = TestRequest::default().uri("/src/client").finish();
    //         let resp = st.handle(&req).respond_to(&req).unwrap();
    //         let resp = resp.as_msg();
    //         assert_eq!(resp.status(), StatusCode::OK);
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_TYPE).unwrap(),
    //             "text/x-rust"
    //         );
    //         assert_eq!(
    //             resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
    //             "inline; filename=\"mod.rs\""
    //         );
    //     }

    //     #[test]
    //     fn integration_serve_index() {
    //         let mut srv = test::TestServer::with_factory(|| {
    //             App::new().handler(
    //                 "test",
    //                 StaticFiles::new(".").index_file("Cargo.toml"),
    //             )
    //         });

    //         let request = srv.get().uri(srv.url("/test")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::OK);
    //         let bytes = srv.execute(response.body()).unwrap();
    //         let data = Bytes::from(fs::read("Cargo.toml").unwrap());
    //         assert_eq!(bytes, data);

    //         let request = srv.get().uri(srv.url("/test/")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::OK);
    //         let bytes = srv.execute(response.body()).unwrap();
    //         let data = Bytes::from(fs::read("Cargo.toml").unwrap());
    //         assert_eq!(bytes, data);

    //         // nonexistent index file
    //         let request = srv.get().uri(srv.url("/test/unknown")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::NOT_FOUND);

    //         let request = srv.get().uri(srv.url("/test/unknown/")).finish().unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::NOT_FOUND);
    //     }

    //     #[test]
    //     fn integration_percent_encoded() {
    //         let mut srv = test::TestServer::with_factory(|| {
    //             App::new().handler(
    //                 "test",
    //                 StaticFiles::new(".").index_file("Cargo.toml"),
    //             )
    //         });

    //         let request = srv
    //             .get()
    //             .uri(srv.url("/test/%43argo.toml"))
    //             .finish()
    //             .unwrap();
    //         let response = srv.execute(request.send()).unwrap();
    //         assert_eq!(response.status(), StatusCode::OK);
    //     }

}

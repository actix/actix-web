#![allow(clippy::borrow_interior_mutable_const, clippy::type_complexity)]

//! Static files support
use std::cell::RefCell;
use std::fmt::Write;
use std::fs::{DirEntry, File};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{cmp, io};

use actix_service::boxed::{self, BoxedNewService, BoxedService};
use actix_service::{IntoNewService, NewService, Service};
use actix_web::dev::{
    AppService, HttpServiceFactory, Payload, ResourceDef, ServiceRequest,
    ServiceResponse,
};
use actix_web::error::{BlockingError, Error, ErrorInternalServerError};
use actix_web::http::header::DispositionType;
use actix_web::{web, FromRequest, HttpRequest, HttpResponse, Responder};
use bytes::Bytes;
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, Poll, Stream};
use mime;
use mime_guess::from_ext;
use percent_encoding::{utf8_percent_encode, CONTROLS};
use v_htmlescape::escape as escape_html_entity;

mod error;
mod named;
mod range;

use self::error::{FilesError, UriSegmentError};
pub use crate::named::NamedFile;
pub use crate::range::HttpRange;

type HttpService = BoxedService<ServiceRequest, ServiceResponse, Error>;
type HttpNewService = BoxedNewService<(), ServiceRequest, ServiceResponse, Error, ()>;

/// Return the MIME type associated with a filename extension (case-insensitive).
/// If `ext` is empty or no associated type for the extension was found, returns
/// the type `application/octet-stream`.
#[inline]
pub fn file_extension_to_mime(ext: &str) -> mime::Mime {
    from_ext(ext).first_or_octet_stream()
}

#[doc(hidden)]
/// A helper created from a `std::fs::File` which reads the file
/// chunk-by-chunk on a `ThreadPool`.
pub struct ChunkedReadFile {
    size: u64,
    offset: u64,
    file: Option<File>,
    fut: Option<Box<dyn Future<Item = (File, Bytes), Error = BlockingError<io::Error>>>>,
    counter: u64,
}

fn handle_error(err: BlockingError<io::Error>) -> Error {
    match err {
        BlockingError::Error(err) => err.into(),
        BlockingError::Canceled => ErrorInternalServerError("Unexpected error"),
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
            self.fut = Some(Box::new(web::block(move || {
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
            })));
            self.poll()
        }
    }
}

type DirectoryRenderer =
    dyn Fn(&Directory, &HttpRequest) -> Result<ServiceResponse, io::Error>;

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
        utf8_percent_encode(&$path.to_string_lossy(), CONTROLS)
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

type MimeOverride = dyn Fn(&mime::Name) -> DispositionType;

/// Static files handling
///
/// `Files` service must be registered with `App::service()` method.
///
/// ```rust
/// use actix_web::App;
/// use actix_files as fs;
///
/// fn main() {
///     let app = App::new()
///         .service(fs::Files::new("/static", "."));
/// }
/// ```
pub struct Files {
    path: String,
    directory: PathBuf,
    index: Option<String>,
    show_index: bool,
    default: Rc<RefCell<Option<Rc<HttpNewService>>>>,
    renderer: Rc<DirectoryRenderer>,
    mime_override: Option<Rc<MimeOverride>>,
    file_flags: named::Flags,
}

impl Clone for Files {
    fn clone(&self) -> Self {
        Self {
            directory: self.directory.clone(),
            index: self.index.clone(),
            show_index: self.show_index,
            default: self.default.clone(),
            renderer: self.renderer.clone(),
            file_flags: self.file_flags,
            path: self.path.clone(),
            mime_override: self.mime_override.clone(),
        }
    }
}

impl Files {
    /// Create new `Files` instance for specified base directory.
    ///
    /// `File` uses `ThreadPool` for blocking filesystem operations.
    /// By default pool with 5x threads of available cpus is used.
    /// Pool size can be changed by setting ACTIX_CPU_POOL environment variable.
    pub fn new<T: Into<PathBuf>>(path: &str, dir: T) -> Files {
        let dir = dir.into().canonicalize().unwrap_or_else(|_| PathBuf::new());
        if !dir.is_dir() {
            log::error!("Specified path is not a directory: {:?}", dir);
        }

        Files {
            path: path.to_string(),
            directory: dir,
            index: None,
            show_index: false,
            default: Rc::new(RefCell::new(None)),
            renderer: Rc::new(directory_listing),
            mime_override: None,
            file_flags: named::Flags::default(),
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

    /// Specifies mime override callback
    pub fn mime_override<F>(mut self, f: F) -> Self
    where
        F: Fn(&mime::Name) -> DispositionType + 'static,
    {
        self.mime_override = Some(Rc::new(f));
        self
    }

    /// Set index file
    ///
    /// Shows specific index file for directory "/" instead of
    /// showing files listing.
    pub fn index_file<T: Into<String>>(mut self, index: T) -> Self {
        self.index = Some(index.into());
        self
    }

    #[inline]
    /// Specifies whether to use ETag or not.
    ///
    /// Default is true.
    pub fn use_etag(mut self, value: bool) -> Self {
        self.file_flags.set(named::Flags::ETAG, value);
        self
    }

    #[inline]
    /// Specifies whether to use Last-Modified or not.
    ///
    /// Default is true.
    pub fn use_last_modified(mut self, value: bool) -> Self {
        self.file_flags.set(named::Flags::LAST_MD, value);
        self
    }

    /// Disable `Content-Disposition` header.
    ///
    /// By default Content-Disposition` header is enabled.
    #[inline]
    pub fn disable_content_disposition(mut self) -> Self {
        self.file_flags.remove(named::Flags::CONTENT_DISPOSITION);
        self
    }

    /// Sets default handler which is used when no matched file could be found.
    pub fn default_handler<F, U>(mut self, f: F) -> Self
    where
        F: IntoNewService<U>,
        U: NewService<
                Config = (),
                Request = ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
            > + 'static,
    {
        // create and configure default resource
        self.default = Rc::new(RefCell::new(Some(Rc::new(boxed::new_service(
            f.into_new_service().map_init_err(|_| ()),
        )))));

        self
    }
}

impl HttpServiceFactory for Files {
    fn register(self, config: &mut AppService) {
        if self.default.borrow().is_none() {
            *self.default.borrow_mut() = Some(config.default_service());
        }
        let rdef = if config.is_root() {
            ResourceDef::root_prefix(&self.path)
        } else {
            ResourceDef::prefix(&self.path)
        };
        config.register_service(rdef, None, self, None)
    }
}

impl NewService for Files {
    type Config = ();
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Service = FilesService;
    type InitError = ();
    type Future = Box<dyn Future<Item = Self::Service, Error = Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        let mut srv = FilesService {
            directory: self.directory.clone(),
            index: self.index.clone(),
            show_index: self.show_index,
            default: None,
            renderer: self.renderer.clone(),
            mime_override: self.mime_override.clone(),
            file_flags: self.file_flags,
        };

        if let Some(ref default) = *self.default.borrow() {
            Box::new(
                default
                    .new_service(&())
                    .map(move |default| {
                        srv.default = Some(default);
                        srv
                    })
                    .map_err(|_| ()),
            )
        } else {
            Box::new(ok(srv))
        }
    }
}

pub struct FilesService {
    directory: PathBuf,
    index: Option<String>,
    show_index: bool,
    default: Option<HttpService>,
    renderer: Rc<DirectoryRenderer>,
    mime_override: Option<Rc<MimeOverride>>,
    file_flags: named::Flags,
}

impl FilesService {
    fn handle_err(
        &mut self,
        e: io::Error,
        req: ServiceRequest,
    ) -> Either<
        FutureResult<ServiceResponse, Error>,
        Box<dyn Future<Item = ServiceResponse, Error = Error>>,
    > {
        log::debug!("Files: Failed to handle {}: {}", req.path(), e);
        if let Some(ref mut default) = self.default {
            default.call(req)
        } else {
            Either::A(ok(req.error_response(e)))
        }
    }
}

impl Service for FilesService {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = Either<
        FutureResult<Self::Response, Self::Error>,
        Box<dyn Future<Item = Self::Response, Error = Self::Error>>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        // let (req, pl) = req.into_parts();

        let real_path = match PathBufWrp::get_pathbuf(req.match_info().path()) {
            Ok(item) => item,
            Err(e) => return Either::A(ok(req.error_response(e))),
        };

        // full filepath
        let path = match self.directory.join(&real_path.0).canonicalize() {
            Ok(path) => path,
            Err(e) => return self.handle_err(e, req),
        };

        if path.is_dir() {
            if let Some(ref redir_index) = self.index {
                let path = path.join(redir_index);

                match NamedFile::open(path) {
                    Ok(mut named_file) => {
                        if let Some(ref mime_override) = self.mime_override {
                            let new_disposition =
                                mime_override(&named_file.content_type.type_());
                            named_file.content_disposition.disposition = new_disposition;
                        }

                        named_file.flags = self.file_flags;
                        let (req, _) = req.into_parts();
                        Either::A(ok(match named_file.respond_to(&req) {
                            Ok(item) => ServiceResponse::new(req, item),
                            Err(e) => ServiceResponse::from_err(e, req),
                        }))
                    }
                    Err(e) => self.handle_err(e, req),
                }
            } else if self.show_index {
                let dir = Directory::new(self.directory.clone(), path);
                let (req, _) = req.into_parts();
                let x = (self.renderer)(&dir, &req);
                match x {
                    Ok(resp) => Either::A(ok(resp)),
                    Err(e) => Either::A(ok(ServiceResponse::from_err(e, req))),
                }
            } else {
                Either::A(ok(ServiceResponse::from_err(
                    FilesError::IsDirectory,
                    req.into_parts().0,
                )))
            }
        } else {
            match NamedFile::open(path) {
                Ok(mut named_file) => {
                    if let Some(ref mime_override) = self.mime_override {
                        let new_disposition =
                            mime_override(&named_file.content_type.type_());
                        named_file.content_disposition.disposition = new_disposition;
                    }

                    named_file.flags = self.file_flags;
                    let (req, _) = req.into_parts();
                    match named_file.respond_to(&req) {
                        Ok(item) => {
                            Either::A(ok(ServiceResponse::new(req.clone(), item)))
                        }
                        Err(e) => Either::A(ok(ServiceResponse::from_err(e, req))),
                    }
                }
                Err(e) => self.handle_err(e, req),
            }
        }
    }
}

#[derive(Debug)]
struct PathBufWrp(PathBuf);

impl PathBufWrp {
    fn get_pathbuf(path: &str) -> Result<Self, UriSegmentError> {
        let mut buf = PathBuf::new();
        for segment in path.split('/') {
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

impl FromRequest for PathBufWrp {
    type Error = UriSegmentError;
    type Future = Result<Self, Self::Error>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        PathBufWrp::get_pathbuf(req.match_info().path())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::iter::FromIterator;
    use std::ops::Add;
    use std::time::{Duration, SystemTime};

    use bytes::BytesMut;

    use super::*;
    use actix_web::http::header::{
        self, ContentDisposition, DispositionParam, DispositionType,
    };
    use actix_web::http::{Method, StatusCode};
    use actix_web::middleware::Compress;
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
    fn test_named_file_content_disposition() {
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
            resp.headers().get(header::CONTENT_DISPOSITION).unwrap(),
            "inline; filename=\"Cargo.toml\""
        );

        let file = NamedFile::open("Cargo.toml")
            .unwrap()
            .disable_content_disposition();
        let req = TestRequest::default().to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert!(resp.headers().get(header::CONTENT_DISPOSITION).is_none());
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
    fn test_mime_override() {
        fn all_attachment(_: &mime::Name) -> DispositionType {
            DispositionType::Attachment
        }

        let mut srv = test::init_service(
            App::new().service(
                Files::new("/", ".")
                    .mime_override(all_attachment)
                    .index_file("Cargo.toml"),
            ),
        );

        let request = TestRequest::get().uri("/").to_request();
        let response = test::call_service(&mut srv, request);
        assert_eq!(response.status(), StatusCode::OK);

        let content_disposition = response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .expect("To have CONTENT_DISPOSITION");
        let content_disposition = content_disposition
            .to_str()
            .expect("Convert CONTENT_DISPOSITION to str");
        assert_eq!(content_disposition, "attachment; filename=\"Cargo.toml\"");
    }

    #[test]
    fn test_named_file_ranges_status_code() {
        let mut srv = test::init_service(
            App::new().service(Files::new("/test", ".").index_file("Cargo.toml")),
        );

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .header(header::RANGE, "bytes=10-20")
            .to_request();
        let response = test::call_service(&mut srv, request);
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);

        // Invalid range header
        let request = TestRequest::get()
            .uri("/t%65st/Cargo.toml")
            .header(header::RANGE, "bytes=1-0")
            .to_request();
        let response = test::call_service(&mut srv, request);

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    }

    #[test]
    fn test_named_file_content_range_headers() {
        let mut srv = test::init_service(
            App::new().service(Files::new("/test", ".").index_file("tests/test.binary")),
        );

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .header(header::RANGE, "bytes=10-20")
            .to_request();

        let response = test::call_service(&mut srv, request);
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
        let response = test::call_service(&mut srv, request);

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
        // use actix_web::body::{MessageBody, ResponseBody};

        let mut srv = test::init_service(
            App::new().service(Files::new("test", ".").index_file("tests/test.binary")),
        );

        // Valid range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .header(header::RANGE, "bytes=10-20")
            .to_request();
        let _response = test::call_service(&mut srv, request);

        // let contentlength = response
        //     .headers()
        //     .get(header::CONTENT_LENGTH)
        //     .unwrap()
        //     .to_str()
        //     .unwrap();
        // assert_eq!(contentlength, "11");

        // Invalid range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .header(header::RANGE, "bytes=10-8")
            .to_request();
        let response = test::call_service(&mut srv, request);
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);

        // Without range header
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            // .no_default_headers()
            .to_request();
        let _response = test::call_service(&mut srv, request);

        // let contentlength = response
        //     .headers()
        //     .get(header::CONTENT_LENGTH)
        //     .unwrap()
        //     .to_str()
        //     .unwrap();
        // assert_eq!(contentlength, "100");

        // chunked
        let request = TestRequest::get()
            .uri("/t%65st/tests/test.binary")
            .to_request();
        let mut response = test::call_service(&mut srv, request);

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
    fn test_head_content_length_headers() {
        let mut srv = test::init_service(
            App::new().service(Files::new("test", ".").index_file("tests/test.binary")),
        );

        // Valid range header
        let request = TestRequest::default()
            .method(Method::HEAD)
            .uri("/t%65st/tests/test.binary")
            .to_request();
        let _response = test::call_service(&mut srv, request);

        // TODO: fix check
        // let contentlength = response
        //     .headers()
        //     .get(header::CONTENT_LENGTH)
        //     .unwrap()
        //     .to_str()
        //     .unwrap();
        // assert_eq!(contentlength, "100");
    }

    #[test]
    fn test_static_files_with_spaces() {
        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").index_file("Cargo.toml")),
        );
        let request = TestRequest::get()
            .uri("/tests/test%20space.binary")
            .to_request();
        let mut response = test::call_service(&mut srv, request);
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

    #[test]
    fn test_named_file_not_allowed() {
        let file = NamedFile::open("Cargo.toml").unwrap();
        let req = TestRequest::default()
            .method(Method::POST)
            .to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        let file = NamedFile::open("Cargo.toml").unwrap();
        let req = TestRequest::default().method(Method::PUT).to_http_request();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_named_file_content_encoding() {
        let mut srv = test::init_service(App::new().wrap(Compress::default()).service(
            web::resource("/").to(|| {
                NamedFile::open("Cargo.toml")
                    .unwrap()
                    .set_content_encoding(header::ContentEncoding::Identity)
            }),
        ));

        let request = TestRequest::get()
            .uri("/")
            .header(header::ACCEPT_ENCODING, "gzip")
            .to_request();
        let res = test::call_service(&mut srv, request);
        assert_eq!(res.status(), StatusCode::OK);
        assert!(!res.headers().contains_key(header::CONTENT_ENCODING));
    }

    #[test]
    fn test_named_file_content_encoding_gzip() {
        let mut srv = test::init_service(App::new().wrap(Compress::default()).service(
            web::resource("/").to(|| {
                NamedFile::open("Cargo.toml")
                    .unwrap()
                    .set_content_encoding(header::ContentEncoding::Gzip)
            }),
        ));

        let request = TestRequest::get()
            .uri("/")
            .header(header::ACCEPT_ENCODING, "gzip")
            .to_request();
        let res = test::call_service(&mut srv, request);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get(header::CONTENT_ENCODING)
                .unwrap()
                .to_str()
                .unwrap(),
            "gzip"
        );
    }

    #[test]
    fn test_named_file_allowed_method() {
        let req = TestRequest::default().method(Method::GET).to_http_request();
        let file = NamedFile::open("Cargo.toml").unwrap();
        let resp = file.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_static_files() {
        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").show_files_listing()),
        );
        let req = TestRequest::with_uri("/missing").to_request();

        let resp = test::call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = test::init_service(App::new().service(Files::new("/", ".")));

        let req = TestRequest::default().to_request();
        let resp = test::call_service(&mut srv, req);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let mut srv = test::init_service(
            App::new().service(Files::new("/", ".").show_files_listing()),
        );
        let req = TestRequest::with_uri("/tests").to_request();
        let mut resp = test::call_service(&mut srv, req);
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
        let _st: Files = Files::new("/", "missing");
        let _st: Files = Files::new("/", "Cargo.toml");
    }

    #[test]
    fn test_default_handler_file_missing() {
        let mut st = test::block_on(
            Files::new("/", ".")
                .default_handler(|req: ServiceRequest| {
                    Ok(req.into_response(HttpResponse::Ok().body("default content")))
                })
                .new_service(&()),
        )
        .unwrap();
        let req = TestRequest::with_uri("/missing").to_srv_request();

        let mut resp = test::call_service(&mut st, req);
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes =
            test::block_on(resp.take_body().fold(BytesMut::new(), |mut b, c| {
                b.extend(c);
                Ok::<_, Error>(b)
            }))
            .unwrap();

        assert_eq!(bytes.freeze(), Bytes::from_static(b"default content"));
    }

    //     #[test]
    //     fn test_serve_index() {
    //         let st = Files::new(".").index_file("test.binary");
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
    //         let st = Files::new(".").index_file("mod.rs");
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
    //                 Files::new(".").index_file("Cargo.toml"),
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
    //                 Files::new(".").index_file("Cargo.toml"),
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

    #[test]
    fn test_path_buf() {
        assert_eq!(
            PathBufWrp::get_pathbuf("/test/.tt").map(|t| t.0),
            Err(UriSegmentError::BadStart('.'))
        );
        assert_eq!(
            PathBufWrp::get_pathbuf("/test/*tt").map(|t| t.0),
            Err(UriSegmentError::BadStart('*'))
        );
        assert_eq!(
            PathBufWrp::get_pathbuf("/test/tt:").map(|t| t.0),
            Err(UriSegmentError::BadEnd(':'))
        );
        assert_eq!(
            PathBufWrp::get_pathbuf("/test/tt<").map(|t| t.0),
            Err(UriSegmentError::BadEnd('<'))
        );
        assert_eq!(
            PathBufWrp::get_pathbuf("/test/tt>").map(|t| t.0),
            Err(UriSegmentError::BadEnd('>'))
        );
        assert_eq!(
            PathBufWrp::get_pathbuf("/seg1/seg2/").unwrap().0,
            PathBuf::from_iter(vec!["seg1", "seg2"])
        );
        assert_eq!(
            PathBufWrp::get_pathbuf("/seg1/../seg2/").unwrap().0,
            PathBuf::from_iter(vec!["seg2"])
        );
    }
}

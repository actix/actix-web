use std::{
    fmt,
    fs::Metadata,
    io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use actix_http::body::AnyBody;
use actix_service::{Service, ServiceFactory};
use actix_web::{
    dev::{
        AppService, BodyEncoding, HttpServiceFactory, ResourceDef, ServiceRequest,
        ServiceResponse, SizedStream,
    },
    http::{
        header::{
            self, Charset, ContentDisposition, DispositionParam, DispositionType, ExtendedValue,
        },
        ContentEncoding, StatusCode,
    },
    Error, HttpMessage, HttpRequest, HttpResponse, Responder,
};
use bitflags::bitflags;
use futures_core::future::LocalBoxFuture;
use mime_guess::from_path;

use crate::{encoding::equiv_utf8_text, range::HttpRange};

bitflags! {
    pub(crate) struct Flags: u8 {
        const ETAG =                0b0000_0001;
        const LAST_MD =             0b0000_0010;
        const CONTENT_DISPOSITION = 0b0000_0100;
        const PREFER_UTF8 =         0b0000_1000;
    }
}

impl Default for Flags {
    fn default() -> Self {
        Flags::from_bits_truncate(0b0000_0111)
    }
}

/// A file with an associated name.
///
/// `NamedFile` can be registered as services:
/// ```
/// use actix_web::App;
/// use actix_files::NamedFile;
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let file = NamedFile::open_async("./static/index.html").await?;
/// let app = App::new().service(file);
/// # Ok(())
/// # }
/// ```
///
/// They can also be returned from handlers:
/// ```
/// use actix_web::{Responder, get};
/// use actix_files::NamedFile;
///
/// #[get("/")]
/// async fn index() -> impl Responder {
///     NamedFile::open_async("./static/index.html").await
/// }
/// ```
pub struct NamedFile {
    path: PathBuf,
    file: File,
    modified: Option<SystemTime>,
    pub(crate) md: Metadata,
    pub(crate) flags: Flags,
    pub(crate) status_code: StatusCode,
    pub(crate) content_type: mime::Mime,
    pub(crate) content_disposition: header::ContentDisposition,
    pub(crate) encoding: Option<ContentEncoding>,
}

impl fmt::Debug for NamedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamedFile")
            .field("path", &self.path)
            .field(
                "file",
                #[cfg(feature = "experimental-io-uring")]
                {
                    &"tokio_uring::File"
                },
                #[cfg(not(feature = "experimental-io-uring"))]
                {
                    &self.file
                },
            )
            .field("modified", &self.modified)
            .field("md", &self.md)
            .field("flags", &self.flags)
            .field("status_code", &self.status_code)
            .field("content_type", &self.content_type)
            .field("content_disposition", &self.content_disposition)
            .field("encoding", &self.encoding)
            .finish()
    }
}

#[cfg(not(feature = "experimental-io-uring"))]
pub(crate) use std::fs::File;
#[cfg(feature = "experimental-io-uring")]
pub(crate) use tokio_uring::fs::File;

impl NamedFile {
    /// Creates an instance from a previously opened file.
    ///
    /// The given `path` need not exist and is only used to determine the `ContentType` and
    /// `ContentDisposition` headers.
    ///
    /// # Examples
    /// ```ignore
    /// use actix_files::NamedFile;
    /// use std::io::{self, Write};
    /// use std::env;
    /// use std::fs::File;
    ///
    /// fn main() -> io::Result<()> {
    ///     let mut file = File::create("foo.txt")?;
    ///     file.write_all(b"Hello, world!")?;
    ///     let named_file = NamedFile::from_file(file, "bar.txt")?;
    ///     # std::fs::remove_file("foo.txt");
    ///     Ok(())
    /// }
    /// ```
    pub fn from_file<P: AsRef<Path>>(file: File, path: P) -> io::Result<NamedFile> {
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
                    ));
                }
            };

            let ct = from_path(&path).first_or_octet_stream();

            let disposition = match ct.type_() {
                mime::IMAGE | mime::TEXT | mime::VIDEO => DispositionType::Inline,
                mime::APPLICATION => match ct.subtype() {
                    mime::JAVASCRIPT | mime::JSON => DispositionType::Inline,
                    name if name == "wasm" => DispositionType::Inline,
                    _ => DispositionType::Attachment,
                },
                _ => DispositionType::Attachment,
            };

            let mut parameters =
                vec![DispositionParam::Filename(String::from(filename.as_ref()))];

            if !filename.is_ascii() {
                parameters.push(DispositionParam::FilenameExt(ExtendedValue {
                    charset: Charset::Ext(String::from("UTF-8")),
                    language_tag: None,
                    value: filename.into_owned().into_bytes(),
                }))
            }

            let cd = ContentDisposition {
                disposition,
                parameters,
            };

            (ct, cd)
        };

        let md = {
            #[cfg(not(feature = "experimental-io-uring"))]
            {
                file.metadata()?
            }

            #[cfg(feature = "experimental-io-uring")]
            {
                use std::os::unix::prelude::{AsRawFd, FromRawFd};

                let fd = file.as_raw_fd();

                // SAFETY: fd is borrowed and lives longer than the unsafe block
                unsafe {
                    let file = std::fs::File::from_raw_fd(fd);
                    let md = file.metadata();
                    // SAFETY: forget the fd before exiting block in success or error case but don't
                    // run destructor (that would close file handle)
                    std::mem::forget(file);
                    md?
                }
            }
        };

        let modified = md.modified().ok();
        let encoding = None;

        Ok(NamedFile {
            path,
            file,
            content_type,
            content_disposition,
            md,
            modified,
            encoding,
            status_code: StatusCode::OK,
            flags: Flags::default(),
        })
    }

    #[cfg(not(feature = "experimental-io-uring"))]
    /// Attempts to open a file in read-only mode.
    ///
    /// # Examples
    /// ```
    /// use actix_files::NamedFile;
    /// let file = NamedFile::open("foo.txt");
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<NamedFile> {
        let file = File::open(&path)?;
        Self::from_file(file, path)
    }

    /// Attempts to open a file asynchronously in read-only mode.
    ///
    /// When the `experimental-io-uring` crate feature is enabled, this will be async.
    /// Otherwise, it will be just like [`open`][Self::open].
    ///
    /// # Examples
    /// ```
    /// use actix_files::NamedFile;
    /// # async fn open() {
    /// let file = NamedFile::open_async("foo.txt").await.unwrap();
    /// # }
    /// ```
    pub async fn open_async<P: AsRef<Path>>(path: P) -> io::Result<NamedFile> {
        let file = {
            #[cfg(not(feature = "experimental-io-uring"))]
            {
                File::open(&path)?
            }

            #[cfg(feature = "experimental-io-uring")]
            {
                File::open(&path).await?
            }
        };

        Self::from_file(file, path)
    }

    /// Returns reference to the underlying `File` object.
    #[inline]
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Retrieve the path of this file.
    ///
    /// # Examples
    /// ```
    /// # use std::io;
    /// use actix_files::NamedFile;
    ///
    /// # async fn path() -> io::Result<()> {
    /// let file = NamedFile::open_async("test.txt").await?;
    /// assert_eq!(file.path().as_os_str(), "foo.txt");
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn path(&self) -> &Path {
        self.path.as_path()
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
    /// sent to the peer.
    ///
    /// By default the disposition is `inline` for `text/*`, `image/*`, `video/*` and
    /// `application/{javascript, json, wasm}` mime types, and `attachment` otherwise,
    /// and the filename is taken from the path provided in the `open` method
    /// after converting it to UTF-8 using.
    /// [`std::ffi::OsStr::to_string_lossy`]
    #[inline]
    pub fn set_content_disposition(mut self, cd: header::ContentDisposition) -> Self {
        self.content_disposition = cd;
        self.flags.insert(Flags::CONTENT_DISPOSITION);
        self
    }

    /// Disable `Content-Disposition` header.
    ///
    /// By default Content-Disposition` header is enabled.
    #[inline]
    pub fn disable_content_disposition(mut self) -> Self {
        self.flags.remove(Flags::CONTENT_DISPOSITION);
        self
    }

    /// Set content encoding for serving this file
    ///
    /// Must be used with [`actix_web::middleware::Compress`] to take effect.
    #[inline]
    pub fn set_content_encoding(mut self, enc: ContentEncoding) -> Self {
        self.encoding = Some(enc);
        self
    }

    /// Specifies whether to use ETag or not.
    ///
    /// Default is true.
    #[inline]
    pub fn use_etag(mut self, value: bool) -> Self {
        self.flags.set(Flags::ETAG, value);
        self
    }

    /// Specifies whether to use Last-Modified or not.
    ///
    /// Default is true.
    #[inline]
    pub fn use_last_modified(mut self, value: bool) -> Self {
        self.flags.set(Flags::LAST_MD, value);
        self
    }

    /// Specifies whether text responses should signal a UTF-8 encoding.
    ///
    /// Default is false (but will default to true in a future version).
    #[inline]
    pub fn prefer_utf8(mut self, value: bool) -> Self {
        self.flags.set(Flags::PREFER_UTF8, value);
        self
    }

    pub(crate) fn etag(&self) -> Option<header::EntityTag> {
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

    pub(crate) fn last_modified(&self) -> Option<header::HttpDate> {
        self.modified.map(|mtime| mtime.into())
    }

    /// Creates an `HttpResponse` with file as a streaming body.
    pub fn into_response(self, req: &HttpRequest) -> HttpResponse {
        if self.status_code != StatusCode::OK {
            let mut res = HttpResponse::build(self.status_code);

            if self.flags.contains(Flags::PREFER_UTF8) {
                let ct = equiv_utf8_text(self.content_type.clone());
                res.insert_header((header::CONTENT_TYPE, ct.to_string()));
            } else {
                res.insert_header((header::CONTENT_TYPE, self.content_type.to_string()));
            }

            if self.flags.contains(Flags::CONTENT_DISPOSITION) {
                res.insert_header((
                    header::CONTENT_DISPOSITION,
                    self.content_disposition.to_string(),
                ));
            }

            if let Some(current_encoding) = self.encoding {
                res.encoding(current_encoding);
            }

            let reader = super::chunked::new_chunked_read(self.md.len(), 0, self.file);

            return res.streaming(reader);
        }

        let etag = if self.flags.contains(Flags::ETAG) {
            self.etag()
        } else {
            None
        };

        let last_modified = if self.flags.contains(Flags::LAST_MD) {
            self.last_modified()
        } else {
            None
        };

        // check preconditions
        let precondition_failed = if !any_match(etag.as_ref(), req) {
            true
        } else if let (Some(ref m), Some(header::IfUnmodifiedSince(ref since))) =
            (last_modified, req.get_header())
        {
            let t1: SystemTime = (*m).into();
            let t2: SystemTime = (*since).into();

            match (t1.duration_since(UNIX_EPOCH), t2.duration_since(UNIX_EPOCH)) {
                (Ok(t1), Ok(t2)) => t1.as_secs() > t2.as_secs(),
                _ => false,
            }
        } else {
            false
        };

        // check last modified
        let not_modified = if !none_match(etag.as_ref(), req) {
            true
        } else if req.headers().contains_key(header::IF_NONE_MATCH) {
            false
        } else if let (Some(ref m), Some(header::IfModifiedSince(ref since))) =
            (last_modified, req.get_header())
        {
            let t1: SystemTime = (*m).into();
            let t2: SystemTime = (*since).into();

            match (t1.duration_since(UNIX_EPOCH), t2.duration_since(UNIX_EPOCH)) {
                (Ok(t1), Ok(t2)) => t1.as_secs() <= t2.as_secs(),
                _ => false,
            }
        } else {
            false
        };

        let mut resp = HttpResponse::build(self.status_code);

        if self.flags.contains(Flags::PREFER_UTF8) {
            let ct = equiv_utf8_text(self.content_type.clone());
            resp.insert_header((header::CONTENT_TYPE, ct.to_string()));
        } else {
            resp.insert_header((header::CONTENT_TYPE, self.content_type.to_string()));
        }

        if self.flags.contains(Flags::CONTENT_DISPOSITION) {
            resp.insert_header((
                header::CONTENT_DISPOSITION,
                self.content_disposition.to_string(),
            ));
        }

        // default compressing
        if let Some(current_encoding) = self.encoding {
            resp.encoding(current_encoding);
        }

        if let Some(lm) = last_modified {
            resp.insert_header((header::LAST_MODIFIED, lm.to_string()));
        }

        if let Some(etag) = etag {
            resp.insert_header((header::ETAG, etag.to_string()));
        }

        resp.insert_header((header::ACCEPT_RANGES, "bytes"));

        let mut length = self.md.len();
        let mut offset = 0;

        // check for range header
        if let Some(ranges) = req.headers().get(header::RANGE) {
            if let Ok(ranges_header) = ranges.to_str() {
                if let Ok(ranges) = HttpRange::parse(ranges_header, length) {
                    length = ranges[0].length;
                    offset = ranges[0].start;

                    resp.encoding(ContentEncoding::Identity);
                    resp.insert_header((
                        header::CONTENT_RANGE,
                        format!("bytes {}-{}/{}", offset, offset + length - 1, self.md.len()),
                    ));
                } else {
                    resp.insert_header((header::CONTENT_RANGE, format!("bytes */{}", length)));
                    return resp.status(StatusCode::RANGE_NOT_SATISFIABLE).finish();
                };
            } else {
                return resp.status(StatusCode::BAD_REQUEST).finish();
            };
        };

        if precondition_failed {
            return resp.status(StatusCode::PRECONDITION_FAILED).finish();
        } else if not_modified {
            return resp
                .status(StatusCode::NOT_MODIFIED)
                .body(AnyBody::<()>::None)
                .map_into_boxed_body();
        }

        let reader = super::chunked::new_chunked_read(length, offset, self.file);

        if offset != 0 || length != self.md.len() {
            resp.status(StatusCode::PARTIAL_CONTENT);
        }

        resp.body(SizedStream::new(length, reader))
    }
}

/// Returns true if `req` has no `If-Match` header or one which matches `etag`.
fn any_match(etag: Option<&header::EntityTag>, req: &HttpRequest) -> bool {
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
fn none_match(etag: Option<&header::EntityTag>, req: &HttpRequest) -> bool {
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

impl Deref for NamedFile {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl DerefMut for NamedFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.file
    }
}

impl Responder for NamedFile {
    fn respond_to(self, req: &HttpRequest) -> HttpResponse {
        self.into_response(req)
    }
}

impl ServiceFactory<ServiceRequest> for NamedFile {
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = NamedFileService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let service = NamedFileService {
            path: self.path.clone(),
        };

        Box::pin(async move { Ok(service) })
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct NamedFileService {
    path: PathBuf,
}

impl Service<ServiceRequest> for NamedFileService {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_service::always_ready!();

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let (req, _) = req.into_parts();

        let path = self.path.clone();
        Box::pin(async move {
            let file = NamedFile::open_async(path).await?;
            let res = file.into_response(&req);
            Ok(ServiceResponse::new(req, res))
        })
    }
}

impl HttpServiceFactory for NamedFile {
    fn register(self, config: &mut AppService) {
        config.register_service(
            ResourceDef::root_prefix(self.path.to_string_lossy().as_ref()),
            None,
            self,
            None,
        )
    }
}

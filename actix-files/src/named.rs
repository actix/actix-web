use std::{
    fs::Metadata,
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use actix_web::{
    body::{self, BoxBody, SizedStream},
    dev::{
        self, AppService, HttpServiceFactory, ResourceDef, Service, ServiceFactory, ServiceRequest,
        ServiceResponse,
    },
    http::{
        header::{
            self, Charset, ContentDisposition, ContentEncoding, DispositionParam, DispositionType,
            ExtendedValue, HeaderValue,
        },
        StatusCode,
    },
    Error, HttpMessage, HttpRequest, HttpResponse, Responder,
};
use bitflags::bitflags;
use derive_more::{Deref, DerefMut};
use futures_core::future::LocalBoxFuture;
use mime::Mime;

use crate::{encoding::equiv_utf8_text, range::HttpRange};

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Flags: u8 {
        const ETAG =                0b0000_0001;
        const LAST_MD =             0b0000_0010;
        const CONTENT_DISPOSITION = 0b0000_0100;
        const PREFER_UTF8 =         0b0000_1000;
    }
}

impl Default for Flags {
    fn default() -> Self {
        Flags::from_bits_truncate(0b0000_1111)
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
#[derive(Debug, Deref, DerefMut)]
pub struct NamedFile {
    #[deref]
    #[deref_mut]
    file: File,
    path: PathBuf,
    modified: Option<SystemTime>,
    pub(crate) md: Metadata,
    pub(crate) flags: Flags,
    pub(crate) status_code: StatusCode,
    pub(crate) content_type: Mime,
    pub(crate) content_disposition: ContentDisposition,
    pub(crate) encoding: Option<ContentEncoding>,
}

#[cfg(not(feature = "experimental-io-uring"))]
pub(crate) use std::fs::File;

#[cfg(feature = "experimental-io-uring")]
pub(crate) use tokio_uring::fs::File;

use super::chunked;

impl NamedFile {
    /// Creates an instance from a previously opened file.
    ///
    /// The given `path` need not exist and is only used to determine the `ContentType` and
    /// `ContentDisposition` headers.
    ///
    /// # Examples
    /// ```ignore
    /// use std::{
    ///     io::{self, Write as _},
    ///     env,
    ///     fs::File
    /// };
    /// use actix_files::NamedFile;
    ///
    /// let mut file = File::create("foo.txt")?;
    /// file.write_all(b"Hello, world!")?;
    /// let named_file = NamedFile::from_file(file, "bar.txt")?;
    /// # std::fs::remove_file("foo.txt");
    /// Ok(())
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

            let ct = mime_guess::from_path(&path).first_or_octet_stream();

            let disposition = match ct.type_() {
                mime::IMAGE | mime::TEXT | mime::AUDIO | mime::VIDEO => DispositionType::Inline,
                mime::APPLICATION => match ct.subtype() {
                    mime::JAVASCRIPT | mime::JSON => DispositionType::Inline,
                    name if name == "wasm" || name == "xhtml" => DispositionType::Inline,
                    _ => DispositionType::Attachment,
                },
                _ => DispositionType::Attachment,
            };

            // replace special characters in filenames which could occur on some filesystems
            let filename_s = filename
                .replace('\n', "%0A") // \n line break
                .replace('\x0B', "%0B") // \v vertical tab
                .replace('\x0C', "%0C") // \f form feed
                .replace('\r', "%0D"); // \r carriage return
            let mut parameters = vec![DispositionParam::Filename(filename_s)];

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

    /// Attempts to open a file in read-only mode.
    ///
    /// # Examples
    /// ```
    /// use actix_files::NamedFile;
    /// let file = NamedFile::open("foo.txt");
    /// ```
    #[cfg(not(feature = "experimental-io-uring"))]
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<NamedFile> {
        let file = File::open(&path)?;
        Self::from_file(file, path)
    }

    /// Attempts to open a file asynchronously in read-only mode.
    ///
    /// When the `experimental-io-uring` crate feature is enabled, this will be async. Otherwise, it
    /// will behave just like `open`.
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

    /// Returns reference to the underlying file object.
    #[inline]
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Returns the filesystem path to this file.
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

    /// Returns the time the file was last modified.
    ///
    /// Returns `None` only on unsupported platforms; see [`std::fs::Metadata::modified()`].
    /// Therefore, it is usually safe to unwrap this.
    #[inline]
    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    /// Returns the filesystem metadata associated with this file.
    #[inline]
    pub fn metadata(&self) -> &Metadata {
        &self.md
    }

    /// Returns the `Content-Type` header that will be used when serving this file.
    #[inline]
    pub fn content_type(&self) -> &Mime {
        &self.content_type
    }

    /// Returns the `Content-Disposition` that will be used when serving this file.
    #[inline]
    pub fn content_disposition(&self) -> &ContentDisposition {
        &self.content_disposition
    }

    /// Returns the `Content-Encoding` that will be used when serving this file.
    ///
    /// A return value of `None` indicates that the content is not already using a compressed
    /// representation and may be subject to compression downstream.
    #[inline]
    pub fn content_encoding(&self) -> Option<ContentEncoding> {
        self.encoding
    }

    /// Set response status code.
    #[deprecated(since = "0.7.0", note = "Prefer `Responder::customize()`.")]
    pub fn set_status_code(mut self, status: StatusCode) -> Self {
        self.status_code = status;
        self
    }

    /// Sets the `Content-Type` header that will be used when serving this file. By default the
    /// `Content-Type` is inferred from the filename extension.
    #[inline]
    pub fn set_content_type(mut self, mime_type: Mime) -> Self {
        self.content_type = mime_type;
        self
    }

    /// Set the Content-Disposition for serving this file. This allows changing the
    /// `inline/attachment` disposition as well as the filename sent to the peer.
    ///
    /// By default the disposition is `inline` for `text/*`, `image/*`, `video/*` and
    /// `application/{javascript, json, wasm}` mime types, and `attachment` otherwise, and the
    /// filename is taken from the path provided in the `open` method after converting it to UTF-8
    /// (using `to_string_lossy`).
    #[inline]
    pub fn set_content_disposition(mut self, cd: ContentDisposition) -> Self {
        self.content_disposition = cd;
        self.flags.insert(Flags::CONTENT_DISPOSITION);
        self
    }

    /// Disables `Content-Disposition` header.
    ///
    /// By default, the `Content-Disposition` header is sent.
    #[inline]
    pub fn disable_content_disposition(mut self) -> Self {
        self.flags.remove(Flags::CONTENT_DISPOSITION);
        self
    }

    /// Sets content encoding for this file.
    ///
    /// This prevents the `Compress` middleware from modifying the file contents and signals to
    /// browsers/clients how to decode it. For example, if serving a compressed HTML file (e.g.,
    /// `index.html.gz`) then use `.set_content_encoding(ContentEncoding::Gzip)`.
    #[inline]
    pub fn set_content_encoding(mut self, enc: ContentEncoding) -> Self {
        self.encoding = Some(enc);
        self
    }

    /// Specifies whether to return `ETag` header in response.
    ///
    /// Default is true.
    #[inline]
    pub fn use_etag(mut self, value: bool) -> Self {
        self.flags.set(Flags::ETAG, value);
        self
    }

    /// Specifies whether to return `Last-Modified` header in response.
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

    /// Creates an `ETag` in a format is similar to Apache's.
    pub(crate) fn etag(&self) -> Option<header::EntityTag> {
        self.modified.as_ref().map(|mtime| {
            let ino = {
                #[cfg(unix)]
                {
                    #[cfg(unix)]
                    use std::os::unix::fs::MetadataExt as _;

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

            header::EntityTag::new_strong(format!(
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
    pub fn into_response(self, req: &HttpRequest) -> HttpResponse<BoxBody> {
        if self.status_code != StatusCode::OK {
            let mut res = HttpResponse::build(self.status_code);

            let ct = if self.flags.contains(Flags::PREFER_UTF8) {
                equiv_utf8_text(self.content_type.clone())
            } else {
                self.content_type
            };

            res.insert_header((header::CONTENT_TYPE, ct.to_string()));

            if self.flags.contains(Flags::CONTENT_DISPOSITION) {
                res.insert_header((
                    header::CONTENT_DISPOSITION,
                    self.content_disposition.to_string(),
                ));
            }

            if let Some(current_encoding) = self.encoding {
                res.insert_header((header::CONTENT_ENCODING, current_encoding.as_str()));
            }

            let reader = chunked::new_chunked_read(self.md.len(), 0, self.file);

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

        let mut res = HttpResponse::build(self.status_code);

        let ct = if self.flags.contains(Flags::PREFER_UTF8) {
            equiv_utf8_text(self.content_type.clone())
        } else {
            self.content_type
        };

        res.insert_header((header::CONTENT_TYPE, ct.to_string()));

        if self.flags.contains(Flags::CONTENT_DISPOSITION) {
            res.insert_header((
                header::CONTENT_DISPOSITION,
                self.content_disposition.to_string(),
            ));
        }

        if let Some(current_encoding) = self.encoding {
            res.insert_header((header::CONTENT_ENCODING, current_encoding.as_str()));
        }

        if let Some(lm) = last_modified {
            res.insert_header((header::LAST_MODIFIED, lm.to_string()));
        }

        if let Some(etag) = etag {
            res.insert_header((header::ETAG, etag.to_string()));
        }

        res.insert_header((header::ACCEPT_RANGES, "bytes"));

        let mut length = self.md.len();
        let mut offset = 0;

        // check for range header
        if let Some(ranges) = req.headers().get(header::RANGE) {
            if let Ok(ranges_header) = ranges.to_str() {
                if let Ok(ranges) = HttpRange::parse(ranges_header, length) {
                    length = ranges[0].length;
                    offset = ranges[0].start;

                    // When a Content-Encoding header is present in a 206 partial content response
                    // for video content, it prevents browser video players from starting playback
                    // before loading the whole video and also prevents seeking.
                    //
                    // See: https://github.com/actix/actix-web/issues/2815
                    //
                    // The assumption of this fix is that the video player knows to not send an
                    // Accept-Encoding header for this request and that downstream middleware will
                    // not attempt compression for requests without it.
                    //
                    // TODO: Solve question around what to do if self.encoding is set and partial
                    // range is requested. Reject request? Ignoring self.encoding seems wrong, too.
                    // In practice, it should not come up.
                    if req.headers().contains_key(&header::ACCEPT_ENCODING) {
                        // don't allow compression middleware to modify partial content
                        res.insert_header((
                            header::CONTENT_ENCODING,
                            HeaderValue::from_static("identity"),
                        ));
                    }

                    res.insert_header((
                        header::CONTENT_RANGE,
                        format!("bytes {}-{}/{}", offset, offset + length - 1, self.md.len()),
                    ));
                } else {
                    res.insert_header((header::CONTENT_RANGE, format!("bytes */{}", length)));
                    return res.status(StatusCode::RANGE_NOT_SATISFIABLE).finish();
                };
            } else {
                return res.status(StatusCode::BAD_REQUEST).finish();
            };
        };

        if precondition_failed {
            return res.status(StatusCode::PRECONDITION_FAILED).finish();
        } else if not_modified {
            return res
                .status(StatusCode::NOT_MODIFIED)
                .body(body::None::new())
                .map_into_boxed_body();
        }

        let reader = chunked::new_chunked_read(length, offset, self.file);

        if offset != 0 || length != self.md.len() {
            res.status(StatusCode::PARTIAL_CONTENT);
        }

        res.body(SizedStream::new(length, reader))
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

impl Responder for NamedFile {
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
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

    dev::always_ready!();

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

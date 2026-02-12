use std::{
    fmt, io,
    ops::Deref,
    path::{Path, PathBuf},
    rc::Rc,
};

use actix_web::{
    body::BoxBody,
    dev::{self, Service, ServiceRequest, ServiceResponse},
    error::Error,
    guard::Guard,
    http::{header, Method},
    HttpResponse,
};
use futures_core::future::LocalBoxFuture;

use crate::{
    named, Directory, DirectoryRenderer, FilesError, HttpService, MimeOverride, NamedFile,
    PathBufWrap, PathFilter,
};

/// Assembled file serving service.
#[derive(Clone)]
pub struct FilesService(pub(crate) Rc<FilesServiceInner>);

impl Deref for FilesService {
    type Target = FilesServiceInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct FilesServiceInner {
    pub(crate) directories: Vec<PathBuf>,
    pub(crate) index: Option<String>,
    pub(crate) show_index: bool,
    pub(crate) redirect_to_slash: bool,
    pub(crate) default: Option<HttpService>,
    pub(crate) renderer: Rc<DirectoryRenderer>,
    pub(crate) mime_override: Option<Rc<MimeOverride>>,
    pub(crate) path_filter: Option<Rc<PathFilter>>,
    pub(crate) file_flags: named::Flags,
    pub(crate) guards: Option<Rc<dyn Guard>>,
    pub(crate) hidden_files: bool,
    pub(crate) try_compressed: bool,
    pub(crate) size_threshold: u64,
    pub(crate) with_permanent_redirect: bool,
}

impl fmt::Debug for FilesServiceInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FilesServiceInner")
    }
}

impl FilesService {
    async fn handle_err(
        &self,
        err: io::Error,
        req: ServiceRequest,
    ) -> Result<ServiceResponse, Error> {
        log::debug!("error handling {}: {}", req.path(), err);

        if let Some(ref default) = self.default {
            default.call(req).await
        } else {
            Ok(req.error_response(err))
        }
    }

    fn serve_named_file_with_encoding(
        &self,
        req: ServiceRequest,
        mut named_file: NamedFile,
        encoding: header::ContentEncoding,
    ) -> ServiceResponse {
        if let Some(ref mime_override) = self.mime_override {
            let new_disposition = mime_override(&named_file.content_type.type_());
            named_file.content_disposition.disposition = new_disposition;
        }
        named_file.flags = self.file_flags;

        let (req, _) = req.into_parts();
        let mut res = named_file
            .read_mode_threshold(self.size_threshold)
            .into_response(&req);

        let header_value = match encoding {
            header::ContentEncoding::Brotli => Some("br"),
            header::ContentEncoding::Gzip => Some("gzip"),
            header::ContentEncoding::Zstd => Some("zstd"),
            header::ContentEncoding::Identity => None,
            // Only variants in SUPPORTED_PRECOMPRESSION_ENCODINGS can occur here
            _ => unreachable!(),
        };
        if let Some(header_value) = header_value {
            res.headers_mut().insert(
                header::CONTENT_ENCODING,
                header::HeaderValue::from_static(header_value),
            );
            // Response representation varies by Accept-Encoding when serving pre-compressed assets.
            res.headers_mut().append(
                header::VARY,
                header::HeaderValue::from_static("accept-encoding"),
            );
        }
        ServiceResponse::new(req, res)
    }

    fn serve_named_file(&self, req: ServiceRequest, named_file: NamedFile) -> ServiceResponse {
        self.serve_named_file_with_encoding(req, named_file, header::ContentEncoding::Identity)
    }

    /// Show index listing for a directory.
    ///
    /// Uses the directory where the path was found as the base directory for index listing.
    fn show_index(&self, req: ServiceRequest, path: PathBuf) -> ServiceResponse {
        let dir = Directory::new(path.clone(), path);

        let (req, _) = req.into_parts();

        (self.renderer)(&dir, &req).unwrap_or_else(|err| ServiceResponse::from_err(err, req))
    }
}

impl fmt::Debug for FilesService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FilesService")
    }
}

impl Service<ServiceRequest> for FilesService {
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    dev::always_ready!();

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let is_method_valid = if let Some(guard) = &self.guards {
            // execute user defined guards
            (**guard).check(&req.guard_ctx())
        } else {
            // default behavior
            matches!(*req.method(), Method::HEAD | Method::GET)
        };

        let this = self.clone();

        Box::pin(async move {
            if !is_method_valid {
                return Ok(req.into_response(
                    HttpResponse::MethodNotAllowed()
                        .insert_header(header::ContentType(mime::TEXT_PLAIN_UTF_8))
                        .body("Request did not meet this resource's requirements."),
                ));
            }

            let path_on_disk =
                match PathBufWrap::parse_path(req.match_info().unprocessed(), this.hidden_files) {
                    Ok(item) => item,
                    Err(err) => return Ok(req.error_response(err)),
                };

            if let Some(filter) = &this.path_filter {
                if !filter(path_on_disk.as_ref(), req.head()) {
                    if let Some(ref default) = this.default {
                        return default.call(req).await;
                    } else {
                        return Ok(req.into_response(HttpResponse::NotFound().finish()));
                    }
                }
            }

            // Try to find file in multiple directories
            let mut found_path = None;
            let mut last_err = None;

            for directory in &this.directories {
                let path = directory.join(&path_on_disk);
                match path.canonicalize() {
                    Ok(canonical_path) => {
                        found_path = Some(canonical_path);
                        break;
                    }
                    Err(err) => {
                        // Keep track of the last error
                        last_err = Some(err);
                    }
                }
            }

            let path = match found_path {
                Some(path) => path,
                None => {
                    // If all directories failed, use the last error
                    let err = last_err.unwrap_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, "File not found")
                    });
                    return this.handle_err(err, req).await;
                }
            };

            // Try serving pre-compressed file even if the uncompressed file doesn't exist yet.
            // Still handle directories (index/listing) through the normal branch below.
            if this.try_compressed && !path.is_dir() {
                if let Some((named_file, encoding)) = find_compressed(&req, &path).await {
                    return Ok(this.serve_named_file_with_encoding(req, named_file, encoding));
                }
            }

            if path.is_dir() {
                if this.redirect_to_slash
                    && !req.path().ends_with('/')
                    && (this.index.is_some() || this.show_index)
                {
                    let redirect_to = format!("{}/", req.path());

                    let response = if this.with_permanent_redirect {
                        HttpResponse::PermanentRedirect()
                    } else {
                        HttpResponse::TemporaryRedirect()
                    }
                    .insert_header((header::LOCATION, redirect_to))
                    .finish();

                    return Ok(req.into_response(response));
                }

                match this.index {
                    Some(ref index) => {
                        let named_path = path.join(index);
                        if this.try_compressed {
                            if let Some((named_file, encoding)) =
                                find_compressed(&req, &named_path).await
                            {
                                return Ok(
                                    this.serve_named_file_with_encoding(req, named_file, encoding)
                                );
                            }
                        }
                        // fallback to the uncompressed version
                        match NamedFile::open_async(named_path).await {
                            Ok(named_file) => Ok(this.serve_named_file(req, named_file)),
                            Err(_) if this.show_index => Ok(this.show_index(req, path)),
                            Err(err) => this.handle_err(err, req).await,
                        }
                    }
                    None if this.show_index => Ok(this.show_index(req, path)),
                    None => Ok(ServiceResponse::from_err(
                        FilesError::IsDirectory,
                        req.into_parts().0,
                    )),
                }
            } else {
                match NamedFile::open_async(&path).await {
                    Ok(named_file) => Ok(this.serve_named_file(req, named_file)),
                    Err(err) => this.handle_err(err, req).await,
                }
            }
        })
    }
}

/// Flate doesn't have an accepted file extension, so it is not included here.
const SUPPORTED_PRECOMPRESSION_ENCODINGS: &[header::ContentEncoding] = &[
    header::ContentEncoding::Brotli,
    header::ContentEncoding::Gzip,
    header::ContentEncoding::Zstd,
    header::ContentEncoding::Identity,
];

/// Searches disk for an acceptable alternate encoding of the content at the given path, as
/// preferred by the request's `Accept-Encoding` header. Returns the corresponding `NamedFile` with
/// the most appropriate supported encoding, if any exist.
async fn find_compressed(
    req: &ServiceRequest,
    original_path: &Path,
) -> Option<(NamedFile, header::ContentEncoding)> {
    use actix_web::HttpMessage;
    use header::{AcceptEncoding, ContentEncoding, Encoding};

    // Retrieve the content type and content disposition based on the original filename. If we
    // can't get these successfully, don't even try to find a compressed file.
    let (content_type, content_disposition) =
        match crate::named::get_content_type_and_disposition(original_path) {
            Ok(values) => values,
            Err(_) => return None,
        };

    let accept_encoding = req.get_header::<AcceptEncoding>()?;

    let mut supported = SUPPORTED_PRECOMPRESSION_ENCODINGS
        .iter()
        .copied()
        .map(Encoding::Known)
        .collect::<Vec<_>>();

    // Only move the original content-type/disposition into the chosen compressed file once.
    let mut content_type = Some(content_type);
    let mut content_disposition = Some(content_disposition);

    loop {
        // Select next acceptable encoding (honouring q=0 rejections) from remaining supported set.
        let chosen = accept_encoding.negotiate(supported.iter())?;

        let encoding = match chosen {
            Encoding::Known(enc) => enc,
            // No supported encoding should ever be unknown here.
            Encoding::Unknown(_) => return None,
        };

        // Identity indicates there is no acceptable pre-compressed representation.
        if encoding == ContentEncoding::Identity {
            return None;
        }

        let extension = match encoding {
            ContentEncoding::Brotli => ".br",
            ContentEncoding::Gzip => ".gz",
            ContentEncoding::Zstd => ".zst",
            ContentEncoding::Identity => unreachable!(),
            // Only variants in SUPPORTED_PRECOMPRESSION_ENCODINGS can occur here.
            _ => unreachable!(),
        };

        let mut compressed_path = original_path.to_owned();
        let mut filename = compressed_path.file_name()?.to_owned();
        filename.push(extension);
        compressed_path.set_file_name(filename);

        match NamedFile::open_async(&compressed_path).await {
            Ok(mut named_file) => {
                named_file.content_type = content_type.take().unwrap();
                named_file.content_disposition = content_disposition.take().unwrap();
                return Some((named_file, encoding));
            }
            // Ignore errors while searching disk for a suitable encoding.
            Err(_) => {
                supported.retain(|enc| enc != &chosen);
            }
        }
    }
}

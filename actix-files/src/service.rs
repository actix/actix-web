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
        f.debug_struct("FilesServiceInner").finish_non_exhaustive()
    }
}

impl FilesService {
    /// Opens a file after checking its resolved path on Windows.
    ///
    /// `directory` is the canonical path of the configured serving root. On Windows, returns
    /// errors from [`Self::canonicalize_path`]. On all platforms, returns file-open errors.
    fn open_named_file(&self, directory: &Path, path: &Path) -> io::Result<NamedFile> {
        #[cfg(windows)]
        self.canonicalize_path(directory, path)?;

        #[cfg(not(windows))]
        let _ = directory;

        NamedFile::open(path)
    }

    /// Resolves a path and applies the hidden-file policy to the resolved names on Windows.
    ///
    /// `directory` must be the canonical path of the configured serving root. When hidden files
    /// are disabled, checks components below that root for a leading dot. If the resolved path
    /// is outside the root, checks all of its components instead.
    ///
    /// Returns [`io::ErrorKind::NotFound`] for a hidden path on Windows, or an error from
    /// [`Path::canonicalize`] if the path cannot be resolved.
    fn canonicalize_path(&self, directory: &Path, path: &Path) -> io::Result<PathBuf> {
        let resolved = path.canonicalize()?;

        #[cfg(windows)]
        if !self.hidden_files {
            // Windows short names can hide a leading dot. Check the resolved names, excluding
            // the configured root. For symlinks outside the root, check the full target path.
            let relative = resolved.strip_prefix(directory).unwrap_or(&resolved);

            if relative.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::Normal(name) if name.as_encoded_bytes().starts_with(b".")
                )
            }) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "hidden file"));
            }
        }

        #[cfg(not(windows))]
        let _ = directory;

        Ok(resolved)
    }

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

    fn show_index(&self, req: ServiceRequest, base: PathBuf, path: PathBuf) -> ServiceResponse {
        let dir = Directory::new(base, path);

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

            let mut last_miss = None;
            let mut first_index_listing = None;
            let mut found_unrenderable_dir = false;

            for directory in &this.directories {
                // full file path
                let path = directory.join(&path_on_disk);

                // Try serving pre-compressed file even if the uncompressed file doesn't exist yet.
                // Still handle directories (index/listing) through the normal branch below.
                if this.try_compressed && !path.is_dir() {
                    if let Some((named_file, encoding)) =
                        find_compressed(&req, &path, &this, directory).await
                    {
                        return Ok(this.serve_named_file_with_encoding(req, named_file, encoding));
                    }
                }

                if let Err(err) = this.canonicalize_path(directory, &path) {
                    if matches!(
                        err.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) {
                        last_miss = Some(err);
                        continue;
                    }

                    return this.handle_err(err, req).await;
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

                    match &this.index {
                        Some(index) => {
                            let named_path = path.join(index);
                            if this.try_compressed {
                                if let Some((named_file, encoding)) =
                                    find_compressed(&req, &named_path, &this, directory).await
                                {
                                    return Ok(this.serve_named_file_with_encoding(
                                        req, named_file, encoding,
                                    ));
                                }
                            }
                            // fallback to the uncompressed version
                            match this.open_named_file(directory, &named_path) {
                                Ok(named_file) => return Ok(this.serve_named_file(req, named_file)),
                                Err(err)
                                    if matches!(
                                        err.kind(),
                                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                                    ) =>
                                {
                                    if this.show_index && first_index_listing.is_none() {
                                        first_index_listing =
                                            Some((directory.to_path_buf(), path.clone()));
                                    }
                                    last_miss = Some(err);
                                }
                                Err(_) if this.show_index => {
                                    if first_index_listing.is_none() {
                                        first_index_listing =
                                            Some((directory.to_path_buf(), path.clone()));
                                    }
                                    break;
                                }
                                Err(err) => return this.handle_err(err, req).await,
                            }
                        }
                        None if this.show_index => {
                            return Ok(this.show_index(req, directory.to_path_buf(), path));
                        }
                        None => found_unrenderable_dir = true,
                    }
                } else {
                    match NamedFile::open(&path) {
                        Ok(named_file) => return Ok(this.serve_named_file(req, named_file)),
                        Err(err)
                            if matches!(
                                err.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                            ) =>
                        {
                            last_miss = Some(err);
                        }
                        Err(err) => return this.handle_err(err, req).await,
                    }
                }
            }

            if let Some((base, path)) = first_index_listing {
                return Ok(this.show_index(req, base, path));
            }

            if found_unrenderable_dir {
                return Ok(ServiceResponse::from_err(
                    FilesError::IsDirectory,
                    req.into_parts().0,
                ));
            }

            let err = last_miss
                .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No such file"));
            this.handle_err(err, req).await
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
    service: &FilesService,
    directory: &Path,
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

        match service.open_named_file(directory, &compressed_path) {
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

#[cfg(all(test, windows))]
mod tests {
    use std::{fs, path::Path, process::Command};

    use actix_web::{
        http::{header, StatusCode},
        test, App,
    };

    use crate::Files;

    fn short_name(path: &Path) -> String {
        let output = Command::new("cmd")
            .args([
                "/D",
                "/C",
                "for %I in (\"%ACTIX_ALIAS_TEST_PATH%\") do @echo %~snxI",
            ])
            .env("ACTIX_ALIAS_TEST_PATH", path)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "Short-name lookup should succeed for {path:?}"
        );

        let name = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        assert!(
            !name.is_empty() && !name.starts_with('.'),
            "Path {path:?} should have a nonempty short name without a leading dot, got {name:?}"
        );
        assert_eq!(
            path.canonicalize().unwrap(),
            path.with_file_name(&name).canonicalize().unwrap(),
            "Short name {name:?} should resolve to {path:?}"
        );

        name
    }

    #[actix_web::test]
    #[ignore = "requires an NTFS temporary directory with 8.3 name creation enabled"]
    async fn hidden_short_names() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".static");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("public.txt"), "public").unwrap();
        fs::write(root.join(".env"), "secret").unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "secret").unwrap();
        fs::write(root.join(".git/index.html"), "secret").unwrap();
        fs::write(root.join(".git/only.gz"), "compressed secret").unwrap();
        fs::create_dir(root.join(".git/empty")).unwrap();
        fs::write(root.join(".config.gz"), "compressed secret").unwrap();

        let env = short_name(&root.join(".env"));
        let git = short_name(&root.join(".git"));
        let compressed = short_name(&root.join(".config.gz"));
        let compressed_stem = Path::new(&compressed)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();

        for allow_hidden in [false, true] {
            let mut files = Files::new("/", &root)
                .index_file("index.html")
                .show_files_listing()
                .try_compressed();

            if allow_hidden {
                files = files.use_hidden_files();
            }

            let app = test::init_service(App::new().service(files)).await;

            let req = test::TestRequest::with_uri("/public.txt").to_request();
            assert_eq!(
                test::call_service(&app, req).await.status(),
                StatusCode::OK,
                "Public files should be served with allow_hidden={allow_hidden}"
            );

            for uri in [
                format!("/{env}"),
                format!("/{git}/config"),
                format!("/{git}/"),
                format!("/{git}/only"),
                format!("/{git}/empty/"),
                format!("/{compressed_stem}"),
            ] {
                let req = test::TestRequest::with_uri(&uri)
                    .insert_header((header::ACCEPT_ENCODING, "gzip"))
                    .to_request();
                let res = test::call_service(&app, req).await;

                assert_eq!(
                    res.status(),
                    if allow_hidden {
                        StatusCode::OK
                    } else {
                        StatusCode::NOT_FOUND
                    },
                    "Request to {uri} should return the expected status for allow_hidden={allow_hidden}"
                );
            }
        }
    }
}

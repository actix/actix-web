use std::{cell::RefCell, fmt, io, path::PathBuf, rc::Rc};

use actix_service::{boxed, IntoServiceFactory, ServiceFactory};
use actix_web::{
    dev::{
        AppService, HttpServiceFactory, ResourceDef, ServiceRequest, ServiceResponse,
    },
    error::Error,
    guard::Guard,
    http::header::DispositionType,
    HttpRequest,
};
use futures_util::future::{ok, FutureExt, LocalBoxFuture};

use crate::{
    directory_listing, named, Directory, DirectoryRenderer, FilesService,
    HttpNewService, MimeOverride,
};

/// Static files handling service.
///
/// `Files` service must be registered with `App::service()` method.
///
/// ```rust
/// use actix_web::App;
/// use actix_files::Files;
///
/// let app = App::new()
///     .service(Files::new("/static", "."));
/// ```
pub struct Files {
    path: String,
    directory: PathBuf,
    index: Option<String>,
    show_index: bool,
    redirect_to_slash: bool,
    default: Rc<RefCell<Option<Rc<HttpNewService>>>>,
    renderer: Rc<DirectoryRenderer>,
    mime_override: Option<Rc<MimeOverride>>,
    file_flags: named::Flags,
    guards: Option<Rc<dyn Guard>>,
}

impl fmt::Debug for Files {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Files")
    }
}

impl Clone for Files {
    fn clone(&self) -> Self {
        Self {
            directory: self.directory.clone(),
            index: self.index.clone(),
            show_index: self.show_index,
            redirect_to_slash: self.redirect_to_slash,
            default: self.default.clone(),
            renderer: self.renderer.clone(),
            file_flags: self.file_flags,
            path: self.path.clone(),
            mime_override: self.mime_override.clone(),
            guards: self.guards.clone(),
        }
    }
}

impl Files {
    /// Create new `Files` instance for a specified base directory.
    ///
    /// # Argument Order
    /// The first argument (`mount_path`) is the root URL at which the static files are served.
    /// For example, `/assets` will serve files at `example.com/assets/...`.
    ///
    /// The second argument (`serve_from`) is the location on disk at which files are loaded.
    /// This can be a relative path. For example, `./` would serve files from the current
    /// working directory.
    ///
    /// # Implementation Notes
    /// If the mount path is set as the root path `/`, services registered after this one will
    /// be inaccessible. Register more specific handlers and services first.
    ///
    /// `Files` uses a threadpool for blocking filesystem operations. By default, the pool uses a
    /// number of threads equal to 5x the number of available logical CPUs. Pool size can be changed
    /// by setting ACTIX_THREADPOOL environment variable.
    pub fn new<T: Into<PathBuf>>(mount_path: &str, serve_from: T) -> Files {
        let orig_dir = serve_from.into();
        let dir = match orig_dir.canonicalize() {
            Ok(canon_dir) => canon_dir,
            Err(_) => {
                log::error!("Specified path is not a directory: {:?}", orig_dir);
                PathBuf::new()
            }
        };

        Files {
            path: mount_path.to_owned(),
            directory: dir,
            index: None,
            show_index: false,
            redirect_to_slash: false,
            default: Rc::new(RefCell::new(None)),
            renderer: Rc::new(directory_listing),
            mime_override: None,
            file_flags: named::Flags::default(),
            guards: None,
        }
    }

    /// Show files listing for directories.
    ///
    /// By default show files listing is disabled.
    pub fn show_files_listing(mut self) -> Self {
        self.show_index = true;
        self
    }

    /// Redirects to a slash-ended path when browsing a directory.
    ///
    /// By default never redirect.
    pub fn redirect_to_slash_directory(mut self) -> Self {
        self.redirect_to_slash = true;
        self
    }

    /// Set custom directory renderer
    pub fn files_listing_renderer<F>(mut self, f: F) -> Self
    where
        for<'r, 's> F: Fn(&'r Directory, &'s HttpRequest) -> Result<ServiceResponse, io::Error>
            + 'static,
    {
        self.renderer = Rc::new(f);
        self
    }

    /// Specifies mime override callback
    pub fn mime_override<F>(mut self, f: F) -> Self
    where
        F: Fn(&mime::Name<'_>) -> DispositionType + 'static,
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

    /// Specifies whether to use ETag or not.
    ///
    /// Default is true.
    #[inline]
    pub fn use_etag(mut self, value: bool) -> Self {
        self.file_flags.set(named::Flags::ETAG, value);
        self
    }

    /// Specifies whether to use Last-Modified or not.
    ///
    /// Default is true.
    #[inline]
    pub fn use_last_modified(mut self, value: bool) -> Self {
        self.file_flags.set(named::Flags::LAST_MD, value);
        self
    }

    /// Specifies whether text responses should signal a UTF-8 encoding.
    ///
    /// Default is false (but will default to true in a future version).
    #[inline]
    pub fn prefer_utf8(mut self, value: bool) -> Self {
        self.file_flags.set(named::Flags::PREFER_UTF8, value);
        self
    }

    /// Specifies custom guards to use for directory listings and files.
    ///
    /// Default behaviour allows GET and HEAD.
    #[inline]
    pub fn use_guards<G: Guard + 'static>(mut self, guards: G) -> Self {
        self.guards = Some(Rc::new(guards));
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
        F: IntoServiceFactory<U>,
        U: ServiceFactory<
                Config = (),
                Request = ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
            > + 'static,
    {
        // create and configure default resource
        self.default = Rc::new(RefCell::new(Some(Rc::new(boxed::factory(
            f.into_factory().map_init_err(|_| ()),
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

impl ServiceFactory for Files {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Config = ();
    type Service = FilesService;
    type InitError = ();
    type Future = LocalBoxFuture<'static, Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: ()) -> Self::Future {
        let mut srv = FilesService {
            directory: self.directory.clone(),
            index: self.index.clone(),
            show_index: self.show_index,
            redirect_to_slash: self.redirect_to_slash,
            default: None,
            renderer: self.renderer.clone(),
            mime_override: self.mime_override.clone(),
            file_flags: self.file_flags,
            guards: self.guards.clone(),
        };

        if let Some(ref default) = *self.default.borrow() {
            default
                .new_service(())
                .map(move |result| match result {
                    Ok(default) => {
                        srv.default = Some(default);
                        Ok(srv)
                    }
                    Err(_) => Err(()),
                })
                .boxed_local()
        } else {
            ok(srv).boxed_local()
        }
    }
}

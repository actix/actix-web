use std::{fmt, io, ops::Deref, path::PathBuf, rc::Rc};

use actix_service::Service;
use actix_web::{
    dev::{ServiceRequest, ServiceResponse},
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
pub struct FilesService(pub Rc<FilesServiceInner>);

impl Deref for FilesService {
    type Target = FilesServiceInner;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

pub struct FilesServiceInner {
    pub(crate) directory: PathBuf,
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

    fn serve_named_file(
        &self,
        req: ServiceRequest,
        mut named_file: NamedFile,
    ) -> ServiceResponse {
        if let Some(ref mime_override) = self.mime_override {
            let new_disposition = mime_override(&named_file.content_type.type_());
            named_file.content_disposition.disposition = new_disposition;
        }
        named_file.flags = self.file_flags;

        let (req, _) = req.into_parts();
        let res = named_file.into_response(&req);
        ServiceResponse::new(req, res)
    }
}

impl fmt::Debug for FilesService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FilesService")
    }
}

impl Service<ServiceRequest> for FilesService {
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<ServiceResponse, Error>>;

    actix_service::always_ready!();

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let is_method_valid = if let Some(guard) = &self.guards {
            // execute user defined guards
            (**guard).check(req.head())
        } else {
            // default behavior
            matches!(*req.method(), Method::HEAD | Method::GET)
        };

        let this = self.clone();

        Box::pin(async move {
            if !is_method_valid {
                return Ok(req.into_response(
                    actix_web::HttpResponse::MethodNotAllowed()
                        .insert_header(header::ContentType(mime::TEXT_PLAIN_UTF_8))
                        .body("Request did not meet this resource's requirements."),
                ));
            }

            let real_path =
                match PathBufWrap::parse_path(req.match_info().path(), this.hidden_files) {
                    Ok(item) => item,
                    Err(e) => return Ok(req.error_response(e)),
                };

            if let Some(filter) = &this.path_filter {
                if !filter(real_path.as_ref(), req.head()) {
                    if let Some(ref default) = this.default {
                        return default.call(req).await;
                    } else {
                        return Ok(
                            req.into_response(actix_web::HttpResponse::NotFound().finish())
                        );
                    }
                }
            }

            // full file path
            let path = this.directory.join(&real_path);
            if let Err(err) = path.canonicalize() {
                return this.handle_err(err, req).await;
            }

            if path.is_dir() {
                if this.redirect_to_slash
                    && !req.path().ends_with('/')
                    && (this.index.is_some() || this.show_index)
                {
                    let redirect_to = format!("{}/", req.path());

                    return Ok(req.into_response(
                        HttpResponse::Found()
                            .insert_header((header::LOCATION, redirect_to))
                            .finish(),
                    ));
                }

                let show_index = |req: ServiceRequest| {
                    let dir = Directory::new(this.directory.clone(), path.clone());

                    let (req, _) = req.into_parts();
                    let x = (this.renderer)(&dir, &req);

                    match x {
                        Ok(resp) => Ok(resp),
                        Err(err) => Ok(ServiceResponse::from_err(err, req)),
                    }
                };

                match this.index {
                    Some(ref index) => {
                        let path = path.join(index);
                        match NamedFile::open_async(path).await {
                            Ok(named_file) => Ok(this.serve_named_file(req, named_file)),
                            Err(_) if this.show_index => show_index(req),
                            Err(err) => this.handle_err(err, req).await,
                        }
                    }
                    None if this.show_index => show_index(req),
                    _ => Ok(ServiceResponse::from_err(
                        FilesError::IsDirectory,
                        req.into_parts().0,
                    )),
                }
            } else {
                match NamedFile::open_async(path).await {
                    Ok(mut named_file) => {
                        if let Some(ref mime_override) = this.mime_override {
                            let new_disposition =
                                mime_override(&named_file.content_type.type_());
                            named_file.content_disposition.disposition = new_disposition;
                        }
                        named_file.flags = this.file_flags;

                        let (req, _) = req.into_parts();
                        let res = named_file.into_response(&req);
                        Ok(ServiceResponse::new(req, res))
                    }
                    Err(err) => this.handle_err(err, req).await,
                }
            }
        })
    }
}

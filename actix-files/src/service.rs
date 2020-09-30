use std::{
    fmt, io,
    path::PathBuf,
    rc::Rc,
    task::{Context, Poll},
};

use actix_service::Service;
use actix_web::{
    dev::{ServiceRequest, ServiceResponse},
    error::Error,
    guard::Guard,
    http::header::{Accept, Header},
    http::{header, Method},
    HttpResponse,
};
use futures_util::future::{ok, Either, LocalBoxFuture, Ready};

use log::debug;

use crate::{
    named, Directory, DirectoryRenderer, FilesError, HttpService, MimeOverride,
    NamedFile, PathBufWrap,
};
use mime::Mime;

use derive_more::{Display, Error};

#[derive(Debug, Display, Error)]
enum FileServiceError {
    #[display(fmt = "mime parsing error")]
    MimeParsingError,
    #[display(fmt = "Provided Mime type too broad")]
    MimeTooBroad,
    #[display(fmt = "Path manipulation error. Failed to add extension to path")]
    PathManipulationError,
}

// Use default implementation for `error_response()` method
impl actix_web::error::ResponseError for FileServiceError {}

/// Assembled file serving service.
pub struct FilesService {
    pub(crate) directory: PathBuf,
    pub(crate) index: Option<String>,
    pub(crate) show_index: bool,
    pub(crate) redirect_to_slash: bool,
    pub(crate) default: Option<HttpService>,
    pub(crate) renderer: Rc<DirectoryRenderer>,
    pub(crate) mime_override: Option<Rc<MimeOverride>>,
    pub(crate) file_flags: named::Flags,
    pub(crate) guards: Option<Rc<dyn Guard>>,
}

type FilesServiceFuture = Either<
    Ready<Result<ServiceResponse, Error>>,
    LocalBoxFuture<'static, Result<ServiceResponse, Error>>,
>;

impl FilesService {
    fn handle_err<T: std::fmt::Display + std::convert::Into<actix_web::Error>>(
        &mut self,
        e: T,
        req: ServiceRequest,
    ) -> FilesServiceFuture {
        log::debug!("Failed to handle {}: {}", req.path(), e);

        if let Some(ref mut default) = self.default {
            Either::Right(default.call(req))
        } else {
            Either::Left(ok(req.error_response(e)))
        }
    }
}

impl fmt::Debug for FilesService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FilesService")
    }
}

impl Service for FilesService {
    type Request = ServiceRequest;
    type Response = ServiceResponse;
    type Error = Error;
    type Future = FilesServiceFuture;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let is_method_valid = if let Some(guard) = &self.guards {
            // execute user defined guards
            (**guard).check(req.head())
        } else {
            // default behavior
            matches!(*req.method(), Method::HEAD | Method::GET)
        };

        if !is_method_valid {
            return Either::Left(ok(req.into_response(
                actix_web::HttpResponse::MethodNotAllowed()
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body("Request did not meet this resource's requirements."),
            )));
        }

        let real_path: PathBufWrap = match req.match_info().path().parse() {
            Ok(item) => item,
            Err(e) => return Either::Left(ok(req.error_response(e))),
        };

        // full file path
        let path = self.directory.join(&real_path);
        debug!("Passed path, non-canonical: {:?}", path);

        // Here we do content negotiation if possible, otherwise we skip
        // We can't do Conneg if the file already has an extension
        // TODO: the Conneg mechanism as of right now does loop over every possible extension for every provided mime type.
        // We should put restrictions in place so this is not a DOS opportunity.
        if path.extension().is_none() {
            match Accept::parse(&req) {
                Ok(ac) => {
                    log::info!("Starting Content Negotiation processing");
                    // Here we clone and sort the vector of MIME types by the provided quality, highest first.
                    let mut mc: actix_web::http::header::Accept = ac.clone();
                    mc.sort_by(|a, b| b.quality.cmp(&a.quality));

                    for item in mc.0 {
                        let mval = &item.item.to_string();
                        if mval == "*/*" {
                            continue;
                        }
                        let eres = mime_guess::get_mime_extensions_str(mval)
                            .ok_or_else(|| FileServiceError::MimeParsingError);

                        match eres {
                            Ok(exts) => {
                                // If more than 5 file extensions for a mime type, it's too broad to test all extensions
                                if exts.len() > 5 {
                                    debug!(
                                        "Warning: more than 5 file exts for mime type"
                                    )
                                }
                                debug!("{:#?}", exts);
                                for extension in exts {
                                    let mut pb = path.clone();

                                    // The following entire section is simply to append the proper extension to the filename. A better way?
                                    let res = pb.components().last().ok_or_else(|| {
                                        FileServiceError::PathManipulationError
                                    });
                                    match res {
                                        Ok(lc) => {
                                            let mut ls = lc.as_os_str().to_owned();
                                            ls.push(".");
                                            ls.push(extension);
                                            pb.pop();
                                            pb.push(ls);
                                        }
                                        Err(e) => {
                                            continue;
                                            // return self.handle_err(e, req);
                                        }
                                    }

                                    match NamedFile::open(pb) {
                                        Ok(mut named_file) => {
                                            if let Some(ref mime_override) =
                                                self.mime_override
                                            {
                                                let new_disposition = mime_override(
                                                    &named_file.content_type.type_(),
                                                );
                                                named_file
                                                    .content_disposition
                                                    .disposition = new_disposition;
                                            }
                                            named_file.flags = self.file_flags;

                                            let (req, _) = req.into_parts();
                                            return match named_file.into_response(&req) {
                                                Ok(item) => Either::Left(ok(
                                                    ServiceResponse::new(
                                                        req.clone(),
                                                        item,
                                                    ),
                                                )),
                                                Err(e) => Either::Left(ok(
                                                    ServiceResponse::from_err(e, req),
                                                )),
                                            };
                                        }
                                        Err(e) => continue,
                                    }
                                }
                                // At this point we've tried all the extensions for a given type
                                // We will move on to other serializations/types
                            }
                            Err(_err) => {
                                debug!("Couldn't retrieve extensions based on the {:?} MIME type, so we skipped it", mval);
                                continue;
                            }
                        }
                    }
                    // TODO: return an error if we've tried all types with no result
                }
                Err(_) => {}
            }
        }

        let path = match path.canonicalize() {
            Ok(path) => path,
            Err(e) => return self.handle_err(e, req),
        };

        if path.is_dir() {
            if let Some(ref redir_index) = self.index {
                if self.redirect_to_slash && !req.path().ends_with('/') {
                    let redirect_to = format!("{}/", req.path());

                    return Either::Left(ok(req.into_response(
                        HttpResponse::Found()
                            .header(header::LOCATION, redirect_to)
                            .body("")
                            .into_body(),
                    )));
                }

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
                        Either::Left(ok(match named_file.into_response(&req) {
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
                    Ok(resp) => Either::Left(ok(resp)),
                    Err(e) => Either::Left(ok(ServiceResponse::from_err(e, req))),
                }
            } else {
                Either::Left(ok(ServiceResponse::from_err(
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
                    match named_file.into_response(&req) {
                        Ok(item) => {
                            Either::Left(ok(ServiceResponse::new(req.clone(), item)))
                        }
                        Err(e) => Either::Left(ok(ServiceResponse::from_err(e, req))),
                    }
                }
                Err(e) => self.handle_err(e, req),
            }
        }
    }
}

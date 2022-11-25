//! Writes a field to a temporary file on disk.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use actix_web::{http::StatusCode, web, Error, HttpRequest, ResponseError};
use derive_more::{Display, Error};
use futures_core::future::LocalBoxFuture;
use futures_util::{FutureExt as _, TryStreamExt as _};
use mime::Mime;
use tempfile_dep::NamedTempFile;
use tokio::io::AsyncWriteExt;

use crate::{
    form::{tempfile::TempfileError::FileIo, FieldReader, Limits},
    Field, MultipartError,
};

/// Write the field to a temporary file on disk.
#[derive(Debug)]
pub struct Tempfile {
    /// The temporary file on disk.
    pub file: NamedTempFile,
    /// The value of the `content-type` header.
    pub content_type: Option<Mime>,
    /// The `filename` value in the `content-disposition` header.
    pub file_name: Option<String>,
    /// The size in bytes of the file.
    pub size: usize,
}

impl<'t> FieldReader<'t> for Tempfile {
    type Future = LocalBoxFuture<'t, Result<Self, MultipartError>>;

    fn read_field(
        req: &'t HttpRequest,
        mut field: Field,
        limits: &'t mut Limits,
    ) -> Self::Future {
        async move {
            let config = TempfileConfig::from_req(req);
            let field_name = field.name().to_owned();
            let mut size = 0;

            let file = if let Some(dir) = &config.directory {
                NamedTempFile::new_in(dir)
            } else {
                NamedTempFile::new()
            }
            .map_err(|e| config.map_error(req, &field_name, FileIo(e)))?;

            let mut file_async = tokio::fs::File::from_std(
                file.reopen()
                    .map_err(|e| config.map_error(req, &field_name, FileIo(e)))?,
            );

            while let Some(chunk) = field.try_next().await? {
                limits.try_consume_limits(chunk.len(), false)?;
                size += chunk.len();
                file_async
                    .write_all(chunk.as_ref())
                    .await
                    .map_err(|e| config.map_error(req, &field_name, FileIo(e)))?;
            }
            file_async
                .flush()
                .await
                .map_err(|e| config.map_error(req, &field_name, FileIo(e)))?;

            Ok(Tempfile {
                file,
                content_type: field.content_type().map(ToOwned::to_owned),
                file_name: field
                    .content_disposition()
                    .get_filename()
                    .map(str::to_owned),
                size,
            })
        }
        .boxed_local()
    }
}

#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum TempfileError {
    /// IO Error
    #[display(fmt = "File I/O error: {}", _0)]
    FileIo(std::io::Error),
}

impl ResponseError for TempfileError {
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// Configuration for the [`Tempfile`] field reader.
#[derive(Clone)]
pub struct TempfileConfig {
    #[allow(clippy::type_complexity)]
    err_handler: Option<Arc<dyn Fn(TempfileError, &HttpRequest) -> Error + Send + Sync>>,
    directory: Option<PathBuf>,
}

const DEFAULT_CONFIG: TempfileConfig = TempfileConfig {
    err_handler: None,
    directory: None,
};

impl TempfileConfig {
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(TempfileError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.err_handler = Some(Arc::new(f));
        self
    }

    /// Extract payload config from app data. Check both `T` and `Data<T>`, in that order, and fall
    /// back to the default payload config.
    fn from_req(req: &HttpRequest) -> &Self {
        req.app_data::<Self>()
            .or_else(|| req.app_data::<web::Data<Self>>().map(|d| d.as_ref()))
            .unwrap_or(&DEFAULT_CONFIG)
    }

    fn map_error(
        &self,
        req: &HttpRequest,
        field_name: &str,
        err: TempfileError,
    ) -> MultipartError {
        let source = if let Some(err_handler) = self.err_handler.as_ref() {
            (*err_handler)(err, req)
        } else {
            err.into()
        };
        MultipartError::Field {
            field_name: field_name.to_owned(),
            source,
        }
    }

    /// Set the directory tempfiles will be created in.
    pub fn directory<P: AsRef<Path>>(mut self, dir: P) -> Self {
        self.directory = Some(dir.as_ref().to_owned());
        self
    }
}

impl Default for TempfileConfig {
    fn default() -> Self {
        DEFAULT_CONFIG
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use actix_http::StatusCode;
    use actix_multipart_rfc7578::client::multipart;
    use actix_web::{web, App, HttpResponse, Responder};

    use crate::form::tempfile::Tempfile;
    use crate::form::tests::send_form;
    use crate::form::MultipartForm;

    #[derive(MultipartForm)]
    struct FileForm {
        file: Tempfile,
    }

    async fn test_file_route(form: MultipartForm<FileForm>) -> impl Responder {
        let mut form = form.into_inner();
        let mut contents = String::new();
        form.file.file.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "Hello, world!");
        assert_eq!(form.file.file_name.unwrap(), "testfile.txt");
        assert_eq!(form.file.content_type.unwrap(), mime::TEXT_PLAIN);
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_file_upload() {
        let srv = actix_test::start(|| App::new().route("/", web::post().to(test_file_route)));

        let mut form = multipart::Form::default();
        let bytes = Cursor::new("Hello, world!");
        form.add_reader_file_with_mime("file", bytes, "testfile.txt", mime::TEXT_PLAIN);
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}

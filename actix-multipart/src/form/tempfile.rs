//! Writes a field to a temporary file on disk.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use actix_web::{http::StatusCode, web, Error, HttpRequest, ResponseError};
use derive_more::{Display, Error};
use futures_core::future::LocalBoxFuture;
use futures_util::TryStreamExt as _;
use mime::Mime;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;

use super::FieldErrorHandler;
use crate::{
    form::{FieldReader, Limits},
    Field, MultipartError,
};

/// Write the field to a temporary file on disk.
#[derive(Debug)]
pub struct TempFile {
    /// The temporary file on disk.
    pub file: NamedTempFile,

    /// The value of the `content-type` header.
    pub content_type: Option<Mime>,

    /// The `filename` value in the `content-disposition` header.
    pub file_name: Option<String>,

    /// The size in bytes of the file.
    pub size: usize,
}

impl<'t> FieldReader<'t> for TempFile {
    type Future = LocalBoxFuture<'t, Result<Self, MultipartError>>;

    fn read_field(req: &'t HttpRequest, mut field: Field, limits: &'t mut Limits) -> Self::Future {
        Box::pin(async move {
            let config = TempFileConfig::from_req(req);
            let mut size = 0;

            let file = config.create_tempfile().map_err(|err| {
                config.map_error(req, &field.form_field_name, TempFileError::FileIo(err))
            })?;

            let mut file_async = tokio::fs::File::from_std(file.reopen().map_err(|err| {
                config.map_error(req, &field.form_field_name, TempFileError::FileIo(err))
            })?);

            while let Some(chunk) = field.try_next().await? {
                limits.try_consume_limits(chunk.len(), false)?;
                size += chunk.len();
                file_async.write_all(chunk.as_ref()).await.map_err(|err| {
                    config.map_error(req, &field.form_field_name, TempFileError::FileIo(err))
                })?;
            }

            file_async.flush().await.map_err(|err| {
                config.map_error(req, &field.form_field_name, TempFileError::FileIo(err))
            })?;

            Ok(TempFile {
                file,
                content_type: field.content_type().map(ToOwned::to_owned),
                file_name: field
                    .content_disposition()
                    .expect("multipart form fields should have a content-disposition header")
                    .get_filename()
                    .map(ToOwned::to_owned),
                size,
            })
        })
    }
}

#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum TempFileError {
    /// File I/O Error
    #[display("File I/O error: {}", _0)]
    FileIo(std::io::Error),
}

impl ResponseError for TempFileError {
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// Configuration for the [`TempFile`] field reader.
#[derive(Clone)]
pub struct TempFileConfig {
    err_handler: FieldErrorHandler<TempFileError>,
    directory: Option<PathBuf>,
}

impl TempFileConfig {
    fn create_tempfile(&self) -> io::Result<NamedTempFile> {
        if let Some(ref dir) = self.directory {
            NamedTempFile::new_in(dir)
        } else {
            NamedTempFile::new()
        }
    }
}

impl TempFileConfig {
    /// Sets custom error handler.
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(TempFileError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.err_handler = Some(Arc::new(f));
        self
    }

    /// Extracts payload config from app data. Check both `T` and `Data<T>`, in that order, and fall
    /// back to the default payload config.
    fn from_req(req: &HttpRequest) -> &Self {
        req.app_data::<Self>()
            .or_else(|| req.app_data::<web::Data<Self>>().map(|d| d.as_ref()))
            .unwrap_or(&DEFAULT_CONFIG)
    }

    fn map_error(&self, req: &HttpRequest, field_name: &str, err: TempFileError) -> MultipartError {
        let source = if let Some(ref err_handler) = self.err_handler {
            (err_handler)(err, req)
        } else {
            err.into()
        };

        MultipartError::Field {
            name: field_name.to_owned(),
            source,
        }
    }

    /// Sets the directory that temp files will be created in.
    ///
    /// The default temporary file location is platform dependent.
    pub fn directory(mut self, dir: impl AsRef<Path>) -> Self {
        self.directory = Some(dir.as_ref().to_owned());
        self
    }
}

const DEFAULT_CONFIG: TempFileConfig = TempFileConfig {
    err_handler: None,
    directory: None,
};

impl Default for TempFileConfig {
    fn default() -> Self {
        DEFAULT_CONFIG
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use actix_multipart_rfc7578::client::multipart;
    use actix_web::{http::StatusCode, web, App, HttpResponse, Responder};

    use crate::form::{tempfile::TempFile, tests::send_form, MultipartForm};

    #[derive(MultipartForm)]
    struct FileForm {
        file: TempFile,
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

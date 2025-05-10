//! Deserializes a field as JSON.

use std::sync::Arc;

use actix_web::{http::StatusCode, web, Error, HttpRequest, ResponseError};
use derive_more::{Deref, DerefMut, Display, Error};
use futures_core::future::LocalBoxFuture;
use serde::de::DeserializeOwned;

use super::FieldErrorHandler;
use crate::{
    form::{bytes::Bytes, FieldReader, Limits},
    Field, MultipartError,
};

/// Deserialize from JSON.
#[derive(Debug, Deref, DerefMut)]
pub struct Json<T: DeserializeOwned>(pub T);

impl<T: DeserializeOwned> Json<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<'t, T> FieldReader<'t> for Json<T>
where
    T: DeserializeOwned + 'static,
{
    type Future = LocalBoxFuture<'t, Result<Self, MultipartError>>;

    fn read_field(req: &'t HttpRequest, field: Field, limits: &'t mut Limits) -> Self::Future {
        Box::pin(async move {
            let config = JsonConfig::from_req(req);

            if config.validate_content_type {
                let valid = if let Some(mime) = field.content_type() {
                    mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON)
                } else {
                    false
                };

                if !valid {
                    return Err(MultipartError::Field {
                        name: field.form_field_name,
                        source: config.map_error(req, JsonFieldError::ContentType),
                    });
                }
            }

            let form_field_name = field.form_field_name.clone();

            let bytes = Bytes::read_field(req, field, limits).await?;

            Ok(Json(serde_json::from_slice(bytes.data.as_ref()).map_err(
                |err| MultipartError::Field {
                    name: form_field_name,
                    source: config.map_error(req, JsonFieldError::Deserialize(err)),
                },
            )?))
        })
    }
}

#[derive(Debug, Display, Error)]
#[non_exhaustive]
pub enum JsonFieldError {
    /// Deserialize error.
    #[display("Json deserialize error: {}", _0)]
    Deserialize(serde_json::Error),

    /// Content type error.
    #[display("Content type error")]
    ContentType,
}

impl ResponseError for JsonFieldError {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

/// Configuration for the [`Json`] field reader.
#[derive(Clone)]
pub struct JsonConfig {
    err_handler: FieldErrorHandler<JsonFieldError>,
    validate_content_type: bool,
}

const DEFAULT_CONFIG: JsonConfig = JsonConfig {
    err_handler: None,
    validate_content_type: true,
};

impl JsonConfig {
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(JsonFieldError, &HttpRequest) -> Error + Send + Sync + 'static,
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

    fn map_error(&self, req: &HttpRequest, err: JsonFieldError) -> Error {
        if let Some(err_handler) = self.err_handler.as_ref() {
            (*err_handler)(err, req)
        } else {
            err.into()
        }
    }

    /// Sets whether or not the field must have a valid `Content-Type` header to be parsed.
    pub fn validate_content_type(mut self, validate_content_type: bool) -> Self {
        self.validate_content_type = validate_content_type;
        self
    }
}

impl Default for JsonConfig {
    fn default() -> Self {
        DEFAULT_CONFIG
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use actix_web::{http::StatusCode, web, web::Bytes, App, HttpResponse, Responder};

    use crate::form::{
        json::{Json, JsonConfig},
        MultipartForm,
    };

    #[derive(MultipartForm)]
    struct JsonForm {
        json: Json<HashMap<String, String>>,
    }

    async fn test_json_route(form: MultipartForm<JsonForm>) -> impl Responder {
        let mut expected = HashMap::new();
        expected.insert("key1".to_owned(), "value1".to_owned());
        expected.insert("key2".to_owned(), "value2".to_owned());
        assert_eq!(&*form.json, &expected);
        HttpResponse::Ok().finish()
    }

    const TEST_JSON: &str = r#"{"key1": "value1", "key2": "value2"}"#;

    #[actix_rt::test]
    async fn test_json_without_content_type() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/", web::post().to(test_json_route))
                .app_data(JsonConfig::default().validate_content_type(false))
        });

        let (body, headers) = crate::test::create_form_data_payload_and_headers(
            "json",
            None,
            None,
            Bytes::from_static(TEST_JSON.as_bytes()),
        );
        let mut req = srv.post("/");
        *req.headers_mut() = headers;
        let res = req.send_body(body).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_content_type_validation() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/", web::post().to(test_json_route))
                .app_data(JsonConfig::default().validate_content_type(true))
        });

        // Deny because wrong content type
        let (body, headers) = crate::test::create_form_data_payload_and_headers(
            "json",
            None,
            Some(mime::APPLICATION_OCTET_STREAM),
            Bytes::from_static(TEST_JSON.as_bytes()),
        );
        let mut req = srv.post("/");
        *req.headers_mut() = headers;
        let res = req.send_body(body).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // Allow because correct content type
        let (body, headers) = crate::test::create_form_data_payload_and_headers(
            "json",
            None,
            Some(mime::APPLICATION_JSON),
            Bytes::from_static(TEST_JSON.as_bytes()),
        );
        let mut req = srv.post("/");
        *req.headers_mut() = headers;
        let res = req.send_body(body).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}

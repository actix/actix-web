//! Extract and process typed data from fields of a `multipart/form-data` request.

use std::{
    any::Any,
    collections::HashMap,
    future::{ready, Future},
    sync::Arc,
};

use actix_web::{dev, error::PayloadError, web, Error, FromRequest, HttpRequest};
use derive_more::{Deref, DerefMut};
use futures_core::future::LocalBoxFuture;
use futures_util::{TryFutureExt as _, TryStreamExt as _};

use crate::{Field, Multipart, MultipartError};

pub mod bytes;
pub mod json;
#[cfg(feature = "tempfile")]
pub mod tempfile;
pub mod text;

#[cfg(feature = "derive")]
pub use actix_multipart_derive::MultipartForm;

type FieldErrorHandler<T> = Option<Arc<dyn Fn(T, &HttpRequest) -> Error + Send + Sync>>;

/// Trait that data types to be used in a multipart form struct should implement.
///
/// It represents an asynchronous handler that processes a multipart field to produce `Self`.
pub trait FieldReader<'t>: Sized + Any {
    /// Future that resolves to a `Self`.
    type Future: Future<Output = Result<Self, MultipartError>>;

    /// The form will call this function to handle the field.
    ///
    /// # Panics
    ///
    /// When reading the `field` payload using its `Stream` implementation, polling (manually or via
    /// `next()`/`try_next()`) may panic after the payload is exhausted. If this is a problem for
    /// your implementation of this method, you should [`fuse()`] the `Field` first.
    ///
    /// [`fuse()`]: futures_util::stream::StreamExt::fuse()
    fn read_field(req: &'t HttpRequest, field: Field, limits: &'t mut Limits) -> Self::Future;
}

/// Used to accumulate the state of the loaded fields.
#[doc(hidden)]
#[derive(Default, Deref, DerefMut)]
pub struct State(pub HashMap<String, Box<dyn Any>>);

/// Trait that the field collection types implement, i.e. `Vec<T>`, `Option<T>`, or `T` itself.
#[doc(hidden)]
pub trait FieldGroupReader<'t>: Sized + Any {
    type Future: Future<Output = Result<(), MultipartError>>;

    /// The form will call this function for each matching field.
    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        duplicate_field: DuplicateField,
    ) -> Self::Future;

    /// Construct `Self` from the group of processed fields.
    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError>;
}

impl<'t, T> FieldGroupReader<'t> for Option<T>
where
    T: FieldReader<'t>,
{
    type Future = LocalBoxFuture<'t, Result<(), MultipartError>>;

    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        duplicate_field: DuplicateField,
    ) -> Self::Future {
        if state.contains_key(&field.form_field_name) {
            match duplicate_field {
                DuplicateField::Ignore => return Box::pin(ready(Ok(()))),

                DuplicateField::Deny => {
                    return Box::pin(ready(Err(MultipartError::DuplicateField(
                        field.form_field_name,
                    ))))
                }

                DuplicateField::Replace => {}
            }
        }

        Box::pin(async move {
            let field_name = field.form_field_name.clone();
            let t = T::read_field(req, field, limits).await?;
            state.insert(field_name, Box::new(t));
            Ok(())
        })
    }

    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError> {
        Ok(state.remove(name).map(|m| *m.downcast::<T>().unwrap()))
    }
}

impl<'t, T> FieldGroupReader<'t> for Vec<T>
where
    T: FieldReader<'t>,
{
    type Future = LocalBoxFuture<'t, Result<(), MultipartError>>;

    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        _duplicate_field: DuplicateField,
    ) -> Self::Future {
        Box::pin(async move {
            // Note: Vec GroupReader always allows duplicates

            let vec = state
                .entry(field.form_field_name.clone())
                .or_insert_with(|| Box::<Vec<T>>::default())
                .downcast_mut::<Vec<T>>()
                .unwrap();

            let item = T::read_field(req, field, limits).await?;
            vec.push(item);

            Ok(())
        })
    }

    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError> {
        Ok(state
            .remove(name)
            .map(|m| *m.downcast::<Vec<T>>().unwrap())
            .unwrap_or_default())
    }
}

impl<'t, T> FieldGroupReader<'t> for T
where
    T: FieldReader<'t>,
{
    type Future = LocalBoxFuture<'t, Result<(), MultipartError>>;

    fn handle_field(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
        duplicate_field: DuplicateField,
    ) -> Self::Future {
        if state.contains_key(&field.form_field_name) {
            match duplicate_field {
                DuplicateField::Ignore => return Box::pin(ready(Ok(()))),

                DuplicateField::Deny => {
                    return Box::pin(ready(Err(MultipartError::DuplicateField(
                        field.form_field_name,
                    ))))
                }

                DuplicateField::Replace => {}
            }
        }

        Box::pin(async move {
            let field_name = field.form_field_name.clone();
            let t = T::read_field(req, field, limits).await?;
            state.insert(field_name, Box::new(t));
            Ok(())
        })
    }

    fn from_state(name: &str, state: &'t mut State) -> Result<Self, MultipartError> {
        state
            .remove(name)
            .map(|m| *m.downcast::<T>().unwrap())
            .ok_or_else(|| MultipartError::MissingField(name.to_owned()))
    }
}

/// Trait that allows a type to be used in the [`struct@MultipartForm`] extractor.
///
/// You should use the [`macro@MultipartForm`] macro to derive this for your struct.
pub trait MultipartCollect: Sized {
    /// An optional limit in bytes to be applied a given field name. Note this limit will be shared
    /// across all fields sharing the same name.
    fn limit(field_name: &str) -> Option<usize>;

    /// The extractor will call this function for each incoming field, the state can be updated
    /// with the processed field data.
    fn handle_field<'t>(
        req: &'t HttpRequest,
        field: Field,
        limits: &'t mut Limits,
        state: &'t mut State,
    ) -> LocalBoxFuture<'t, Result<(), MultipartError>>;

    /// Once all the fields have been processed and stored in the state, this is called
    /// to convert into the struct representation.
    fn from_state(state: State) -> Result<Self, MultipartError>;
}

#[doc(hidden)]
pub enum DuplicateField {
    /// Additional fields are not processed.
    Ignore,

    /// An error will be raised.
    Deny,

    /// All fields will be processed, the last one will replace all previous.
    Replace,
}

/// Used to keep track of the remaining limits for the form and current field.
pub struct Limits {
    pub total_limit_remaining: usize,
    pub memory_limit_remaining: usize,
    pub field_limit_remaining: Option<usize>,
}

impl Limits {
    pub fn new(total_limit: usize, memory_limit: usize) -> Self {
        Self {
            total_limit_remaining: total_limit,
            memory_limit_remaining: memory_limit,
            field_limit_remaining: None,
        }
    }

    /// This function should be called within a [`FieldReader`] when reading each chunk of a field
    /// to ensure that the form limits are not exceeded.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The number of bytes being read from this chunk
    /// * `in_memory` - Whether to consume from the memory limits
    pub fn try_consume_limits(
        &mut self,
        bytes: usize,
        in_memory: bool,
    ) -> Result<(), MultipartError> {
        self.total_limit_remaining = self
            .total_limit_remaining
            .checked_sub(bytes)
            .ok_or(MultipartError::Payload(PayloadError::Overflow))?;

        if in_memory {
            self.memory_limit_remaining = self
                .memory_limit_remaining
                .checked_sub(bytes)
                .ok_or(MultipartError::Payload(PayloadError::Overflow))?;
        }

        if let Some(field_limit) = self.field_limit_remaining {
            self.field_limit_remaining = Some(
                field_limit
                    .checked_sub(bytes)
                    .ok_or(MultipartError::Payload(PayloadError::Overflow))?,
            );
        }

        Ok(())
    }
}

/// Typed `multipart/form-data` extractor.
///
/// To extract typed data from a multipart stream, the inner type `T` must implement the
/// [`MultipartCollect`] trait. You should use the [`macro@MultipartForm`] macro to derive this
/// for your struct.
///
/// Note that this extractor rejects requests with any other Content-Type such as `multipart/mixed`,
/// `multipart/related`, or non-multipart media types.
///
/// Add a [`MultipartFormConfig`] to your app data to configure extraction.
#[derive(Deref, DerefMut)]
pub struct MultipartForm<T: MultipartCollect>(pub T);

impl<T: MultipartCollect> MultipartForm<T> {
    /// Unwrap into inner `T` value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> FromRequest for MultipartForm<T>
where
    T: MultipartCollect + 'static,
{
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let mut multipart = Multipart::from_req(req, payload);

        let content_type = match multipart.content_type_or_bail() {
            Ok(content_type) => content_type,
            Err(err) => return Box::pin(ready(Err(err.into()))),
        };

        if content_type.subtype() != mime::FORM_DATA {
            // this extractor only supports multipart/form-data
            return Box::pin(ready(Err(MultipartError::ContentTypeIncompatible.into())));
        };

        let config = MultipartFormConfig::from_req(req);
        let mut limits = Limits::new(config.total_limit, config.memory_limit);

        let req = req.clone();
        let req2 = req.clone();
        let err_handler = config.err_handler.clone();

        Box::pin(
            async move {
                let mut state = State::default();

                // ensure limits are shared for all fields with this name
                let mut field_limits = HashMap::<String, Option<usize>>::new();

                while let Some(field) = multipart.try_next().await? {
                    debug_assert!(
                        !field.form_field_name.is_empty(),
                        "multipart form fields should have names",
                    );

                    // Retrieve the limit for this field
                    let entry = field_limits
                        .entry(field.form_field_name.clone())
                        .or_insert_with(|| T::limit(&field.form_field_name));

                    limits.field_limit_remaining.clone_from(entry);

                    T::handle_field(&req, field, &mut limits, &mut state).await?;

                    // Update the stored limit
                    *entry = limits.field_limit_remaining;
                }

                let inner = T::from_state(state)?;
                Ok(MultipartForm(inner))
            }
            .map_err(move |err| {
                if let Some(handler) = err_handler {
                    (*handler)(err, &req2)
                } else {
                    err.into()
                }
            }),
        )
    }
}

type MultipartFormErrorHandler =
    Option<Arc<dyn Fn(MultipartError, &HttpRequest) -> Error + Send + Sync>>;

/// [`struct@MultipartForm`] extractor configuration.
///
/// Add to your app data to have it picked up by [`struct@MultipartForm`] extractors.
#[derive(Clone)]
pub struct MultipartFormConfig {
    total_limit: usize,
    memory_limit: usize,
    err_handler: MultipartFormErrorHandler,
}

impl MultipartFormConfig {
    /// Sets maximum accepted payload size for the entire form. By default this limit is 50MiB.
    pub fn total_limit(mut self, total_limit: usize) -> Self {
        self.total_limit = total_limit;
        self
    }

    /// Sets maximum accepted data that will be read into memory. By default this limit is 2MiB.
    pub fn memory_limit(mut self, memory_limit: usize) -> Self {
        self.memory_limit = memory_limit;
        self
    }

    /// Sets custom error handler.
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(MultipartError, &HttpRequest) -> Error + Send + Sync + 'static,
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
}

const DEFAULT_CONFIG: MultipartFormConfig = MultipartFormConfig {
    total_limit: 52_428_800, // 50 MiB
    memory_limit: 2_097_152, // 2 MiB
    err_handler: None,
};

impl Default for MultipartFormConfig {
    fn default() -> Self {
        DEFAULT_CONFIG
    }
}

#[cfg(test)]
mod tests {
    use actix_http::encoding::Decoder;
    use actix_multipart_rfc7578::client::multipart;
    use actix_test::TestServer;
    use actix_web::{
        dev::Payload, http::StatusCode, web, App, HttpRequest, HttpResponse, Resource, Responder,
    };
    use awc::{Client, ClientResponse};
    use futures_core::future::LocalBoxFuture;
    use futures_util::TryStreamExt as _;

    use super::MultipartForm;
    use crate::{
        form::{
            bytes::Bytes, tempfile::TempFile, text::Text, FieldReader, Limits, MultipartFormConfig,
        },
        Field, MultipartError,
    };

    pub async fn send_form(
        srv: &TestServer,
        form: multipart::Form<'static>,
        uri: &'static str,
    ) -> ClientResponse<Decoder<Payload>> {
        Client::default()
            .post(srv.url(uri))
            .content_type(form.content_type())
            .send_body(multipart::Body::from(form))
            .await
            .unwrap()
    }

    /// Test `Option` fields.
    #[derive(MultipartForm)]
    struct TestOptions {
        field1: Option<Text<String>>,
        field2: Option<Text<String>>,
    }

    async fn test_options_route(form: MultipartForm<TestOptions>) -> impl Responder {
        assert!(form.field1.is_some());
        assert!(form.field2.is_none());
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_options() {
        let srv = actix_test::start(|| App::new().route("/", web::post().to(test_options_route)));

        let mut form = multipart::Form::default();
        form.add_text("field1", "value");

        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test `Vec` fields.
    #[derive(MultipartForm)]
    struct TestVec {
        list1: Vec<Text<String>>,
        list2: Vec<Text<String>>,
    }

    async fn test_vec_route(form: MultipartForm<TestVec>) -> impl Responder {
        let form = form.into_inner();
        let strings = form
            .list1
            .into_iter()
            .map(|s| s.into_inner())
            .collect::<Vec<_>>();
        assert_eq!(strings, vec!["value1", "value2", "value3"]);
        assert_eq!(form.list2.len(), 0);
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_vec() {
        let srv = actix_test::start(|| App::new().route("/", web::post().to(test_vec_route)));

        let mut form = multipart::Form::default();
        form.add_text("list1", "value1");
        form.add_text("list1", "value2");
        form.add_text("list1", "value3");

        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the `rename` field attribute.
    #[derive(MultipartForm)]
    struct TestFieldRenaming {
        #[multipart(rename = "renamed")]
        field1: Text<String>,
        #[multipart(rename = "field1")]
        field2: Text<String>,
        field3: Text<String>,
    }

    async fn test_field_renaming_route(form: MultipartForm<TestFieldRenaming>) -> impl Responder {
        assert_eq!(&*form.field1, "renamed");
        assert_eq!(&*form.field2, "field1");
        assert_eq!(&*form.field3, "field3");
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_field_renaming() {
        let srv =
            actix_test::start(|| App::new().route("/", web::post().to(test_field_renaming_route)));

        let mut form = multipart::Form::default();
        form.add_text("renamed", "renamed");
        form.add_text("field1", "field1");
        form.add_text("field3", "field3");

        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the `deny_unknown_fields` struct attribute.
    #[derive(MultipartForm)]
    #[multipart(deny_unknown_fields)]
    struct TestDenyUnknown {}

    #[derive(MultipartForm)]
    struct TestAllowUnknown {}

    async fn test_deny_unknown_route(_: MultipartForm<TestDenyUnknown>) -> impl Responder {
        HttpResponse::Ok().finish()
    }

    async fn test_allow_unknown_route(_: MultipartForm<TestAllowUnknown>) -> impl Responder {
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_deny_unknown() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/deny", web::post().to(test_deny_unknown_route))
                .route("/allow", web::post().to(test_allow_unknown_route))
        });

        let mut form = multipart::Form::default();
        form.add_text("unknown", "value");
        let response = send_form(&srv, form, "/deny").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let mut form = multipart::Form::default();
        form.add_text("unknown", "value");
        let response = send_form(&srv, form, "/allow").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the `duplicate_field` struct attribute.
    #[derive(MultipartForm)]
    #[multipart(duplicate_field = "deny")]
    struct TestDuplicateDeny {
        _field: Text<String>,
    }

    #[derive(MultipartForm)]
    #[multipart(duplicate_field = "replace")]
    struct TestDuplicateReplace {
        field: Text<String>,
    }

    #[derive(MultipartForm)]
    #[multipart(duplicate_field = "ignore")]
    struct TestDuplicateIgnore {
        field: Text<String>,
    }

    async fn test_duplicate_deny_route(_: MultipartForm<TestDuplicateDeny>) -> impl Responder {
        HttpResponse::Ok().finish()
    }

    async fn test_duplicate_replace_route(
        form: MultipartForm<TestDuplicateReplace>,
    ) -> impl Responder {
        assert_eq!(&*form.field, "second_value");
        HttpResponse::Ok().finish()
    }

    async fn test_duplicate_ignore_route(
        form: MultipartForm<TestDuplicateIgnore>,
    ) -> impl Responder {
        assert_eq!(&*form.field, "first_value");
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_duplicate_field() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/deny", web::post().to(test_duplicate_deny_route))
                .route("/replace", web::post().to(test_duplicate_replace_route))
                .route("/ignore", web::post().to(test_duplicate_ignore_route))
        });

        let mut form = multipart::Form::default();
        form.add_text("_field", "first_value");
        form.add_text("_field", "second_value");
        let response = send_form(&srv, form, "/deny").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let mut form = multipart::Form::default();
        form.add_text("field", "first_value");
        form.add_text("field", "second_value");
        let response = send_form(&srv, form, "/replace").await;
        assert_eq!(response.status(), StatusCode::OK);

        let mut form = multipart::Form::default();
        form.add_text("field", "first_value");
        form.add_text("field", "second_value");
        let response = send_form(&srv, form, "/ignore").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Test the Limits.
    #[derive(MultipartForm)]
    struct TestMemoryUploadLimits {
        field: Bytes,
    }

    #[derive(MultipartForm)]
    struct TestFileUploadLimits {
        field: TempFile,
    }

    async fn test_upload_limits_memory(
        form: MultipartForm<TestMemoryUploadLimits>,
    ) -> impl Responder {
        assert!(!form.field.data.is_empty());
        HttpResponse::Ok().finish()
    }

    async fn test_upload_limits_file(form: MultipartForm<TestFileUploadLimits>) -> impl Responder {
        assert!(form.field.size > 0);
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_memory_limits() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/text", web::post().to(test_upload_limits_memory))
                .route("/file", web::post().to(test_upload_limits_file))
                .app_data(
                    MultipartFormConfig::default()
                        .memory_limit(20)
                        .total_limit(usize::MAX),
                )
        });

        // Exceeds the 20 byte memory limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/text").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Memory limit should not apply when the data is being streamed to disk
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/file").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_total_limit() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/text", web::post().to(test_upload_limits_memory))
                .route("/file", web::post().to(test_upload_limits_file))
                .app_data(
                    MultipartFormConfig::default()
                        .memory_limit(usize::MAX)
                        .total_limit(20),
                )
        });

        // Within the 20 byte limit
        let mut form = multipart::Form::default();
        form.add_text("field", "7 bytes");
        let response = send_form(&srv, form, "/text").await;
        assert_eq!(response.status(), StatusCode::OK);

        // Exceeds the 20 byte overall limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/text").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Exceeds the 20 byte overall limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/file").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[derive(MultipartForm)]
    struct TestFieldLevelLimits {
        #[multipart(limit = "30B")]
        field: Vec<Bytes>,
    }

    async fn test_field_level_limits_route(
        form: MultipartForm<TestFieldLevelLimits>,
    ) -> impl Responder {
        assert!(!form.field.is_empty());
        HttpResponse::Ok().finish()
    }

    #[actix_rt::test]
    async fn test_field_level_limits() {
        let srv = actix_test::start(|| {
            App::new()
                .route("/", web::post().to(test_field_level_limits_route))
                .app_data(
                    MultipartFormConfig::default()
                        .memory_limit(usize::MAX)
                        .total_limit(usize::MAX),
                )
        });

        // Within the 30 byte limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);

        // Exceeds the the 30 byte limit
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is more than 30 bytes long");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // Total of values (14 bytes) is within 30 byte limit for "field"
        let mut form = multipart::Form::default();
        form.add_text("field", "7 bytes");
        form.add_text("field", "7 bytes");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::OK);

        // Total of values exceeds 30 byte limit for "field"
        let mut form = multipart::Form::default();
        form.add_text("field", "this string is 28 bytes long");
        form.add_text("field", "this string is 28 bytes long");
        let response = send_form(&srv, form, "/").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_rt::test]
    async fn non_multipart_form_data() {
        #[derive(MultipartForm)]
        struct TestNonMultipartFormData {
            #[allow(unused)]
            #[multipart(limit = "30B")]
            foo: Text<String>,
        }

        async fn non_multipart_form_data_route(
            _form: MultipartForm<TestNonMultipartFormData>,
        ) -> String {
            unreachable!("request is sent with multipart/mixed");
        }

        let srv = actix_test::start(|| {
            App::new().route("/", web::post().to(non_multipart_form_data_route))
        });

        let mut form = multipart::Form::default();
        form.add_text("foo", "foo");

        // mangle content-type, keeping the boundary
        let ct = form.content_type().replacen("/form-data", "/mixed", 1);

        let res = Client::default()
            .post(srv.url("/"))
            .content_type(ct)
            .send_body(multipart::Body::from(form))
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[should_panic(expected = "called `Result::unwrap()` on an `Err` value: Connect(Disconnected)")]
    #[actix_web::test]
    async fn field_try_next_panic() {
        #[derive(Debug)]
        struct NullSink;

        impl<'t> FieldReader<'t> for NullSink {
            type Future = LocalBoxFuture<'t, Result<Self, MultipartError>>;

            fn read_field(
                _: &'t HttpRequest,
                mut field: Field,
                _limits: &'t mut Limits,
            ) -> Self::Future {
                Box::pin(async move {
                    // exhaust field stream
                    while let Some(_chunk) = field.try_next().await? {}

                    // poll again, crash
                    let _post = field.try_next().await;

                    Ok(Self)
                })
            }
        }

        #[allow(dead_code)]
        #[derive(MultipartForm)]
        struct NullSinkForm {
            foo: NullSink,
        }

        async fn null_sink(_form: MultipartForm<NullSinkForm>) -> impl Responder {
            "unreachable"
        }

        let srv = actix_test::start(|| App::new().service(Resource::new("/").post(null_sink)));

        let mut form = multipart::Form::default();
        form.add_text("foo", "data is not important to this test");

        // panics with Err(Connect(Disconnected)) due to form NullSink panic
        let _res = send_form(&srv, form, "/").await;
    }
}

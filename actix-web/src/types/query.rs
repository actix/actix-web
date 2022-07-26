//! For query parameter extractor documentation, see [`Query`].

use std::{fmt, ops, sync::Arc};

use actix_utils::future::{err, ok, Ready};
use serde::de::DeserializeOwned;

use crate::{dev::Payload, error::QueryPayloadError, Error, FromRequest, HttpRequest};

/// Extract typed information from the request's query.
///
/// To extract typed data from the URL query string, the inner type `T` must implement the
/// [`DeserializeOwned`] trait.
///
/// Use [`QueryConfig`] to configure extraction process.
///
/// # Panics
/// A query string consists of unordered `key=value` pairs, therefore it cannot be decoded into any
/// type which depends upon data ordering (eg. tuples). Trying to do so will result in a panic.
///
/// # Examples
/// ```
/// use actix_web::{get, web};
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize)]
/// pub enum ResponseType {
///    Token,
///    Code
/// }
///
/// #[derive(Debug, Deserialize)]
/// pub struct AuthRequest {
///    id: u64,
///    response_type: ResponseType,
/// }
///
/// // Deserialize `AuthRequest` struct from query string.
/// // This handler gets called only if the request's query parameters contain both fields.
/// // A valid request path for this handler would be `/?id=64&response_type=Code"`.
/// #[get("/")]
/// async fn index(info: web::Query<AuthRequest>) -> String {
///     format!("Authorization request for id={} and type={:?}!", info.id, info.response_type)
/// }
///
/// // To access the entire underlying query struct, use `.into_inner()`.
/// #[get("/debug1")]
/// async fn debug1(info: web::Query<AuthRequest>) -> String {
///     dbg!("Authorization object = {:?}", info.into_inner());
///     "OK".to_string()
/// }
///
/// // Or use destructuring, which is equivalent to `.into_inner()`.
/// #[get("/debug2")]
/// async fn debug2(web::Query(info): web::Query<AuthRequest>) -> String {
///     dbg!("Authorization object = {:?}", info);
///     "OK".to_string()
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Query<T>(pub T);

impl<T> Query<T> {
    /// Unwrap into inner `T` value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: DeserializeOwned> Query<T> {
    /// Deserialize a `T` from the URL encoded query parameter string.
    ///
    /// ```
    /// # use std::collections::HashMap;
    /// # use actix_web::web::Query;
    /// let numbers = Query::<HashMap<String, u32>>::from_query("one=1&two=2").unwrap();
    /// assert_eq!(numbers.get("one"), Some(&1));
    /// assert_eq!(numbers.get("two"), Some(&2));
    /// assert!(numbers.get("three").is_none());
    /// ```
    pub fn from_query(query_str: &str) -> Result<Self, QueryPayloadError> {
        serde_urlencoded::from_str::<T>(query_str)
            .map(Self)
            .map_err(QueryPayloadError::Deserialize)
    }
}

impl<T> ops::Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> ops::DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: fmt::Display> fmt::Display for Query<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// See [here](#Examples) for example of usage as an extractor.
impl<T: DeserializeOwned> FromRequest for Query<T> {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let error_handler = req
            .app_data::<QueryConfig>()
            .and_then(|c| c.err_handler.clone());

        serde_urlencoded::from_str::<T>(req.query_string())
            .map(|val| ok(Query(val)))
            .unwrap_or_else(move |e| {
                let e = QueryPayloadError::Deserialize(e);

                log::debug!(
                    "Failed during Query extractor deserialization. \
                     Request path: {:?}",
                    req.path()
                );

                let e = if let Some(error_handler) = error_handler {
                    (error_handler)(e, req)
                } else {
                    e.into()
                };

                err(e)
            })
    }
}

/// Query extractor configuration.
///
/// # Examples
/// ```
/// use actix_web::{error, get, web, App, FromRequest, HttpResponse};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's querystring
/// #[get("/")]
/// async fn index(info: web::Query<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// // custom `Query` extractor configuration
/// let query_cfg = web::QueryConfig::default()
///     // use custom error handler
///     .error_handler(|err, req| {
///         error::InternalError::from_response(err, HttpResponse::Conflict().finish()).into()
///     });
///
/// App::new()
///     .app_data(query_cfg)
///     .service(index);
/// ```
#[derive(Clone, Default)]
pub struct QueryConfig {
    #[allow(clippy::type_complexity)]
    err_handler: Option<Arc<dyn Fn(QueryPayloadError, &HttpRequest) -> Error + Send + Sync>>,
}

impl QueryConfig {
    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(QueryPayloadError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.err_handler = Some(Arc::new(f));
        self
    }
}

#[cfg(test)]
mod tests {
    use actix_http::StatusCode;
    use derive_more::Display;
    use serde::Deserialize;

    use super::*;
    use crate::{error::InternalError, test::TestRequest, HttpResponse};

    #[derive(Deserialize, Debug, Display)]
    struct Id {
        id: String,
    }

    #[actix_rt::test]
    async fn test_service_request_extract() {
        let req = TestRequest::with_uri("/name/user1/").to_srv_request();
        assert!(Query::<Id>::from_query(req.query_string()).is_err());

        let req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();
        let mut s = Query::<Id>::from_query(req.query_string()).unwrap();

        assert_eq!(s.id, "test");
        assert_eq!(
            format!("{}, {:?}", s, s),
            "test, Query(Id { id: \"test\" })"
        );

        s.id = "test1".to_string();
        let s = s.into_inner();
        assert_eq!(s.id, "test1");
    }

    #[actix_rt::test]
    async fn test_request_extract() {
        let req = TestRequest::with_uri("/name/user1/").to_srv_request();
        let (req, mut pl) = req.into_parts();
        assert!(Query::<Id>::from_request(&req, &mut pl).await.is_err());

        let req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();
        let (req, mut pl) = req.into_parts();

        let mut s = Query::<Id>::from_request(&req, &mut pl).await.unwrap();
        assert_eq!(s.id, "test");
        assert_eq!(
            format!("{}, {:?}", s, s),
            "test, Query(Id { id: \"test\" })"
        );

        s.id = "test1".to_string();
        let s = s.into_inner();
        assert_eq!(s.id, "test1");
    }

    #[actix_rt::test]
    #[should_panic]
    async fn test_tuple_panic() {
        let req = TestRequest::with_uri("/?one=1&two=2").to_srv_request();
        let (req, mut pl) = req.into_parts();

        Query::<(u32, u32)>::from_request(&req, &mut pl)
            .await
            .unwrap();
    }

    #[actix_rt::test]
    async fn test_custom_error_responder() {
        let req = TestRequest::with_uri("/name/user1/")
            .app_data(QueryConfig::default().error_handler(|e, _| {
                let resp = HttpResponse::UnprocessableEntity().finish();
                InternalError::from_response(e, resp).into()
            }))
            .to_srv_request();

        let (req, mut pl) = req.into_parts();
        let query = Query::<Id>::from_request(&req, &mut pl).await;

        assert!(query.is_err());
        assert_eq!(
            query
                .unwrap_err()
                .as_response_error()
                .error_response()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

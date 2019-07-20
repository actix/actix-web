//! Query extractor

use std::sync::Arc;
use std::{fmt, ops};

use actix_http::error::Error;
use serde::de;
use serde_urlencoded;

use crate::dev::Payload;
use crate::error::QueryPayloadError;
use crate::extract::FromRequest;
use crate::request::HttpRequest;

/// Extract typed information from the request's query.
///
/// **Note**: A query string consists of unordered `key=value` pairs, therefore it cannot
/// be decoded into any type which depends upon data ordering e.g. tuples or tuple-structs.
/// Attempts to do so will *fail at runtime*.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App};
///
/// #[derive(Debug, Deserialize)]
/// pub enum ResponseType {
///    Token,
///    Code
/// }
///
/// #[derive(Deserialize)]
/// pub struct AuthRequest {
///    id: u64,
///    response_type: ResponseType,
/// }
///
/// // Use `Query` extractor for query information (and destructure it within the signature).
/// // This handler gets called only if the request's query string contains a `username` field.
/// // The correct request for this handler would be `/index.html?id=64&response_type=Code"`.
/// fn index(web::Query(info): web::Query<AuthRequest>) -> String {
///     format!("Authorization request for client with id={} and type={:?}!", info.id, info.response_type)
/// }
///
/// fn main() {
///     let app = App::new().service(
///        web::resource("/index.html").route(web::get().to(index))); // <- use `Query` extractor
/// }
/// ```
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Query<T>(pub T);

impl<T> Query<T> {
    /// Deconstruct to a inner value
    pub fn into_inner(self) -> T {
        self.0
    }

    /// Get query parameters from the path
    pub fn from_query(query_str: &str) -> Result<Self, QueryPayloadError>
    where
        T: de::DeserializeOwned,
    {
        serde_urlencoded::from_str::<T>(query_str)
            .map(|val| Ok(Query(val)))
            .unwrap_or_else(move |e| Err(QueryPayloadError::Deserialize(e)))
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

impl<T: fmt::Debug> fmt::Debug for Query<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for Query<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Extract typed information from the request's query.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App};
///
/// #[derive(Debug, Deserialize)]
/// pub enum ResponseType {
///    Token,
///    Code
/// }
///
/// #[derive(Deserialize)]
/// pub struct AuthRequest {
///    id: u64,
///    response_type: ResponseType,
/// }
///
/// // Use `Query` extractor for query information.
/// // This handler get called only if request's query contains `username` field
/// // The correct request for this handler would be `/index.html?id=64&response_type=Code"`
/// fn index(info: web::Query<AuthRequest>) -> String {
///     format!("Authorization request for client with id={} and type={:?}!", info.id, info.response_type)
/// }
///
/// fn main() {
///     let app = App::new().service(
///        web::resource("/index.html")
///            .route(web::get().to(index))); // <- use `Query` extractor
/// }
/// ```
impl<T> FromRequest for Query<T>
where
    T: de::DeserializeOwned,
{
    type Error = Error;
    type Future = Result<Self, Error>;
    type Config = QueryConfig;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let error_handler = req
            .app_data::<Self::Config>()
            .map(|c| c.ehandler.clone())
            .unwrap_or(None);

        serde_urlencoded::from_str::<T>(req.query_string())
            .map(|val| Ok(Query(val)))
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

                Err(e)
            })
    }
}

/// Query extractor configuration
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{error, web, App, FromRequest, HttpResponse};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's querystring
/// fn index(info: web::Query<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").data(
///             // change query extractor configuration
///             web::Query::<Info>::configure(|cfg| {
///                 cfg.error_handler(|err, req| {  // <- create custom error response
///                     error::InternalError::from_response(
///                         err, HttpResponse::Conflict().finish()).into()
///                 })
///             }))
///             .route(web::post().to(index))
///     );
/// }
/// ```
#[derive(Clone)]
pub struct QueryConfig {
    ehandler:
        Option<Arc<dyn Fn(QueryPayloadError, &HttpRequest) -> Error + Send + Sync>>,
}

impl QueryConfig {
    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(QueryPayloadError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.ehandler = Some(Arc::new(f));
        self
    }
}

impl Default for QueryConfig {
    fn default() -> Self {
        QueryConfig { ehandler: None }
    }
}

#[cfg(test)]
mod tests {
    use actix_http::http::StatusCode;
    use derive_more::Display;
    use serde_derive::Deserialize;

    use super::*;
    use crate::error::InternalError;
    use crate::test::TestRequest;
    use crate::HttpResponse;

    #[derive(Deserialize, Debug, Display)]
    struct Id {
        id: String,
    }

    #[test]
    fn test_service_request_extract() {
        let req = TestRequest::with_uri("/name/user1/").to_srv_request();
        assert!(Query::<Id>::from_query(&req.query_string()).is_err());

        let req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();
        let mut s = Query::<Id>::from_query(&req.query_string()).unwrap();

        assert_eq!(s.id, "test");
        assert_eq!(format!("{}, {:?}", s, s), "test, Id { id: \"test\" }");

        s.id = "test1".to_string();
        let s = s.into_inner();
        assert_eq!(s.id, "test1");
    }

    #[test]
    fn test_request_extract() {
        let req = TestRequest::with_uri("/name/user1/").to_srv_request();
        let (req, mut pl) = req.into_parts();
        assert!(Query::<Id>::from_request(&req, &mut pl).is_err());

        let req = TestRequest::with_uri("/name/user1/?id=test").to_srv_request();
        let (req, mut pl) = req.into_parts();

        let mut s = Query::<Id>::from_request(&req, &mut pl).unwrap();
        assert_eq!(s.id, "test");
        assert_eq!(format!("{}, {:?}", s, s), "test, Id { id: \"test\" }");

        s.id = "test1".to_string();
        let s = s.into_inner();
        assert_eq!(s.id, "test1");
    }

    #[test]
    fn test_custom_error_responder() {
        let req = TestRequest::with_uri("/name/user1/")
            .data(QueryConfig::default().error_handler(|e, _| {
                let resp = HttpResponse::UnprocessableEntity().finish();
                InternalError::from_response(e, resp).into()
            }))
            .to_srv_request();

        let (req, mut pl) = req.into_parts();
        let query = Query::<Id>::from_request(&req, &mut pl);

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

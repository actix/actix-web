//! Query extractor

use std::{fmt, ops};

use actix_http::error::Error;
use serde::de;
use serde_urlencoded;

use crate::dev::Payload;
use crate::extract::FromRequest;
use crate::request::HttpRequest;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
/// Extract typed information from from the request's query.
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
///        web::resource("/index.html").route(web::get().to(index))); // <- use `Query` extractor
/// }
/// ```
pub struct Query<T>(T);

impl<T> Query<T> {
    /// Deconstruct to a inner value
    pub fn into_inner(self) -> T {
        self.0
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

/// Extract typed information from from the request's query.
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
    type Config = ();
    type Error = Error;
    type Future = Result<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        serde_urlencoded::from_str::<T>(req.query_string())
            .map(|val| Ok(Query(val)))
            .unwrap_or_else(|e| {
                log::debug!(
                    "Failed during Query extractor deserialization. \
                     Request path: {:?}",
                    req.path()
                );
                Err(e.into())
            })
    }
}

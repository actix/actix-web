//! For header extractor helper documentation, see [`Header`](crate::types::Header).

use std::{fmt, ops};

use futures_util::future::{err, ok, Ready};

use crate::dev::Payload;
use crate::error::ParseError;
use crate::extract::FromRequest;
use crate::http::header::Header as ParseHeader;
use crate::HttpRequest;

/// Extract typed headers from the request.
///
/// To extract a header, the inner type `T` must implement the
/// [`Header`](crate::http::header::Header) trait.
///
/// # Examples
/// ```
/// use actix_web::{get, web, http::header};
///
/// #[get("/")]
/// async fn index(date: web::Header<header::Date>) -> String {
///     format!("Request was sent at {}", date.to_string())
/// }
/// ```
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Header<T>(pub T);

impl<T> Header<T> {
    /// Unwrap into the inner `T` value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> ops::Deref for Header<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> ops::DerefMut for Header<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> fmt::Debug for Header<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Header: {:?}", self.0)
    }
}

impl<T> fmt::Display for Header<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<T> FromRequest for Header<T>
where
    T: ParseHeader,
{
    type Error = ParseError;
    type Future = Ready<Result<Self, Self::Error>>;
    type Config = ();

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        match ParseHeader::parse(req) {
            Ok(header) => ok(Header(header)),
            Err(e) => err(e),
        }
    }
}

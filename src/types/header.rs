use std::{fmt, ops};

use futures_util::future::{err, ok, Ready};

use crate::dev::Payload;
use crate::error::ParseError;
use crate::extract::FromRequest;
use crate::http::header;
use crate::HttpRequest;

/// Header extractor and responder.
pub struct Header<T>(pub T);

impl<T> Header<T> {
    /// Unwrap into inner `T` value.
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

/// See [here](#extractor) for example of usage as an extractor.
impl<T> FromRequest for Header<T>
where
    T: header::Header,
{
    type Error = ParseError;
    type Future = Ready<Result<Self, Self::Error>>;
    type Config = ();

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        match header::Header::parse(req) {
            Ok(header) => ok(Header(header)),
            Err(e) => err(e),
        }
    }
}

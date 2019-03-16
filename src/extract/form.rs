//! Form extractor

use std::rc::Rc;
use std::{fmt, ops};

use actix_http::dev::UrlEncoded;
use actix_http::error::{Error, UrlencodedError};
use bytes::Bytes;
use futures::{Future, Stream};
use serde::de::DeserializeOwned;

use crate::extract::FromRequest;
use crate::request::HttpRequest;
use crate::service::ServiceFromRequest;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
/// Extract typed information from the request's body.
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**FormConfig**](struct.FormConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App};
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// Extract form data using serde.
/// /// This handler get called only if content type is *x-www-form-urlencoded*
/// /// and content of the request could be deserialized to a `FormData` struct
/// fn index(form: web::Form<FormData>) -> String {
///     format!("Welcome {}!", form.username)
/// }
/// # fn main() {}
/// ```
pub struct Form<T>(pub T);

impl<T> Form<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> ops::Deref for Form<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> ops::DerefMut for Form<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, P> FromRequest<P> for Form<T>
where
    T: DeserializeOwned + 'static,
    P: Stream<Item = Bytes, Error = crate::error::PayloadError> + 'static,
{
    type Error = Error;
    type Future = Box<Future<Item = Self, Error = Error>>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let req2 = req.clone();
        let (limit, err) = req
            .load_config::<FormConfig>()
            .map(|c| (c.limit, c.ehandler.clone()))
            .unwrap_or((16384, None));

        Box::new(
            UrlEncoded::new(req)
                .limit(limit)
                .map_err(move |e| {
                    if let Some(err) = err {
                        (*err)(e, &req2)
                    } else {
                        e.into()
                    }
                })
                .map(Form),
        )
    }
}

impl<T: fmt::Debug> fmt::Debug for Form<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for Form<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Form extractor configuration
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, Result};
///
/// #[derive(Deserialize)]
/// struct FormData {
///     username: String,
/// }
///
/// /// Extract form data using serde.
/// /// Custom configuration is used for this handler, max payload size is 4k
/// fn index(form: web::Form<FormData>) -> Result<String> {
///     Ok(format!("Welcome {}!", form.username))
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html")
///             .route(web::get()
///                 // change `Form` extractor configuration
///                 .data(web::FormConfig::default().limit(4097))
///                 .to(index))
///     );
/// }
/// ```
#[derive(Clone)]
pub struct FormConfig {
    limit: usize,
    ehandler: Option<Rc<Fn(UrlencodedError, &HttpRequest) -> Error>>,
}

impl FormConfig {
    /// Change max size of payload. By default max size is 16Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(UrlencodedError, &HttpRequest) -> Error + 'static,
    {
        self.ehandler = Some(Rc::new(f));
        self
    }
}

impl Default for FormConfig {
    fn default() -> Self {
        FormConfig {
            limit: 16384,
            ehandler: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_http::http::header;
    use bytes::Bytes;
    use serde_derive::Deserialize;

    use super::*;
    use crate::test::{block_on, TestRequest};

    #[derive(Deserialize, Debug, PartialEq)]
    struct Info {
        hello: String,
    }

    #[test]
    fn test_form() {
        let mut req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(header::CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world"))
        .to_from();

        let s = block_on(Form::<Info>::from_request(&mut req)).unwrap();
        assert_eq!(s.hello, "world");
    }
}

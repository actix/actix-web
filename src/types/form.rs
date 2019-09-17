//! Form extractor

use std::rc::Rc;
use std::{fmt, ops};

use actix_http::{Error, HttpMessage, Payload, Response};
use bytes::BytesMut;
use encoding_rs::{Encoding, UTF_8};
use futures::{Future, Poll, Stream};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::dev::Decompress;
use crate::error::UrlencodedError;
use crate::extract::FromRequest;
use crate::http::{
    header::{ContentType, CONTENT_LENGTH},
    StatusCode,
};
use crate::request::HttpRequest;
use crate::responder::Responder;

/// Form data helper (`application/x-www-form-urlencoded`)
///
/// Can be use to extract url-encoded data from the request body,
/// or send url-encoded data as the response.
///
/// ## Extract
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**FormConfig**](struct.FormConfig.html) allows to configure extraction
/// process.
///
/// ### Example
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
///
/// ## Respond
///
/// The `Form` type also allows you to respond with well-formed url-encoded data:
/// simply return a value of type Form<T> where T is the type to be url-encoded.
/// The type  must implement `serde::Serialize`;
///
/// ### Example
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// #
/// #[derive(Serialize)]
/// struct SomeForm {
///     name: String,
///     age: u8
/// }
///
/// // Will return a 200 response with header
/// // `Content-Type: application/x-www-form-urlencoded`
/// // and body "name=actix&age=123"
/// fn index() -> web::Form<SomeForm> {
///     web::Form(SomeForm {
///         name: "actix".into(),
///         age: 123
///     })
/// }
/// # fn main() {}
/// ```
#[derive(PartialEq, Eq, PartialOrd, Ord)]
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

impl<T> FromRequest for Form<T>
where
    T: DeserializeOwned + 'static,
{
    type Config = FormConfig;
    type Error = Error;
    type Future = Box<dyn Future<Item = Self, Error = Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let req2 = req.clone();
        let (limit, err) = req
            .app_data::<FormConfig>()
            .map(|c| (c.limit, c.ehandler.clone()))
            .unwrap_or((16384, None));

        Box::new(
            UrlEncoded::new(req, payload)
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

impl<T: Serialize> Responder for Form<T> {
    type Error = Error;
    type Future = Result<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let body = match serde_urlencoded::to_string(&self.0) {
            Ok(body) => body,
            Err(e) => return Err(e.into()),
        };

        Ok(Response::build(StatusCode::OK)
            .set(ContentType::form_url_encoded())
            .body(body))
    }
}

/// Form extractor configuration
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App, FromRequest, Result};
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
///             // change `Form` extractor configuration
///             .data(
///                 web::Form::<FormData>::configure(|cfg| cfg.limit(4097))
///             )
///             .route(web::get().to(index))
///     );
/// }
/// ```
#[derive(Clone)]
pub struct FormConfig {
    limit: usize,
    ehandler: Option<Rc<dyn Fn(UrlencodedError, &HttpRequest) -> Error>>,
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

/// Future that resolves to a parsed urlencoded values.
///
/// Parse `application/x-www-form-urlencoded` encoded request's body.
/// Return `UrlEncoded` future. Form can be deserialized to any type that
/// implements `Deserialize` trait from *serde*.
///
/// Returns error:
///
/// * content type is not `application/x-www-form-urlencoded`
/// * content-length is greater than 32k
///
pub struct UrlEncoded<U> {
    stream: Option<Decompress<Payload>>,
    limit: usize,
    length: Option<usize>,
    encoding: &'static Encoding,
    err: Option<UrlencodedError>,
    fut: Option<Box<dyn Future<Item = U, Error = UrlencodedError>>>,
}

impl<U> UrlEncoded<U> {
    /// Create a new future to URL encode a request
    pub fn new(req: &HttpRequest, payload: &mut Payload) -> UrlEncoded<U> {
        // check content type
        if req.content_type().to_lowercase() != "application/x-www-form-urlencoded" {
            return Self::err(UrlencodedError::ContentType);
        }
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(_) => return Self::err(UrlencodedError::ContentType),
        };

        let mut len = None;
        if let Some(l) = req.headers().get(&CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                } else {
                    return Self::err(UrlencodedError::UnknownLength);
                }
            } else {
                return Self::err(UrlencodedError::UnknownLength);
            }
        };

        let payload = Decompress::from_headers(payload.take(), req.headers());
        UrlEncoded {
            encoding,
            stream: Some(payload),
            limit: 32_768,
            length: len,
            fut: None,
            err: None,
        }
    }

    fn err(e: UrlencodedError) -> Self {
        UrlEncoded {
            stream: None,
            limit: 32_768,
            fut: None,
            err: Some(e),
            length: None,
            encoding: UTF_8,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl<U> Future for UrlEncoded<U>
where
    U: DeserializeOwned + 'static,
{
    type Item = U;
    type Error = UrlencodedError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        // payload size
        let limit = self.limit;
        if let Some(len) = self.length.take() {
            if len > limit {
                return Err(UrlencodedError::Overflow { size: len, limit });
            }
        }

        // future
        let encoding = self.encoding;
        let fut = self
            .stream
            .take()
            .unwrap()
            .from_err()
            .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                if (body.len() + chunk.len()) > limit {
                    Err(UrlencodedError::Overflow {
                        size: body.len() + chunk.len(),
                        limit,
                    })
                } else {
                    body.extend_from_slice(&chunk);
                    Ok(body)
                }
            })
            .and_then(move |body| {
                if encoding == UTF_8 {
                    serde_urlencoded::from_bytes::<U>(&body)
                        .map_err(|_| UrlencodedError::Parse)
                } else {
                    let body = encoding
                        .decode_without_bom_handling_and_without_replacement(&body)
                        .map(|s| s.into_owned())
                        .ok_or(UrlencodedError::Parse)?;
                    serde_urlencoded::from_str::<U>(&body)
                        .map_err(|_| UrlencodedError::Parse)
                }
            });
        self.fut = Some(Box::new(fut));
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::http::header::{HeaderValue, CONTENT_TYPE};
    use crate::test::{block_on, TestRequest};

    #[derive(Deserialize, Serialize, Debug, PartialEq)]
    struct Info {
        hello: String,
        counter: i64,
    }

    #[test]
    fn test_form() {
        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "11")
                .set_payload(Bytes::from_static(b"hello=world&counter=123"))
                .to_http_parts();

        let Form(s) = block_on(Form::<Info>::from_request(&req, &mut pl)).unwrap();
        assert_eq!(
            s,
            Info {
                hello: "world".into(),
                counter: 123
            }
        );
    }

    fn eq(err: UrlencodedError, other: UrlencodedError) -> bool {
        match err {
            UrlencodedError::Overflow { .. } => match other {
                UrlencodedError::Overflow { .. } => true,
                _ => false,
            },
            UrlencodedError::UnknownLength => match other {
                UrlencodedError::UnknownLength => true,
                _ => false,
            },
            UrlencodedError::ContentType => match other {
                UrlencodedError::ContentType => true,
                _ => false,
            },
            _ => false,
        }
    }

    #[test]
    fn test_urlencoded_error() {
        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "xxxx")
                .to_http_parts();
        let info = block_on(UrlEncoded::<Info>::new(&req, &mut pl));
        assert!(eq(info.err().unwrap(), UrlencodedError::UnknownLength));

        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "1000000")
                .to_http_parts();
        let info = block_on(UrlEncoded::<Info>::new(&req, &mut pl));
        assert!(eq(
            info.err().unwrap(),
            UrlencodedError::Overflow { size: 0, limit: 0 }
        ));

        let (req, mut pl) = TestRequest::with_header(CONTENT_TYPE, "text/plain")
            .header(CONTENT_LENGTH, "10")
            .to_http_parts();
        let info = block_on(UrlEncoded::<Info>::new(&req, &mut pl));
        assert!(eq(info.err().unwrap(), UrlencodedError::ContentType));
    }

    #[test]
    fn test_urlencoded() {
        let (req, mut pl) =
            TestRequest::with_header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(CONTENT_LENGTH, "11")
                .set_payload(Bytes::from_static(b"hello=world&counter=123"))
                .to_http_parts();

        let info = block_on(UrlEncoded::<Info>::new(&req, &mut pl)).unwrap();
        assert_eq!(
            info,
            Info {
                hello: "world".to_owned(),
                counter: 123
            }
        );

        let (req, mut pl) = TestRequest::with_header(
            CONTENT_TYPE,
            "application/x-www-form-urlencoded; charset=utf-8",
        )
        .header(CONTENT_LENGTH, "11")
        .set_payload(Bytes::from_static(b"hello=world&counter=123"))
        .to_http_parts();

        let info = block_on(UrlEncoded::<Info>::new(&req, &mut pl)).unwrap();
        assert_eq!(
            info,
            Info {
                hello: "world".to_owned(),
                counter: 123
            }
        );
    }

    #[test]
    fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let form = Form(Info {
            hello: "world".to_string(),
            counter: 123,
        });
        let resp = form.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("application/x-www-form-urlencoded")
        );

        use crate::responder::tests::BodyTest;
        assert_eq!(resp.body().bin_ref(), b"hello=world&counter=123");
    }
}

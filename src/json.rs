use bytes::BytesMut;
use futures::{Future, Poll, Stream};
use http::header::CONTENT_LENGTH;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use mime;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json;

use error::{Error, JsonPayloadError};
use handler::{FromRequest, Responder};
use http::StatusCode;
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Json helper
///
/// Json can be used for two different purpose. First is for json response
/// generation and second is for extracting typed information from request's
/// payload.
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**JsonConfig**](dev/struct.JsonConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{App, Json, Result, http};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// fn index(info: Json<Info>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html",
///        |r| r.method(http::Method::POST).with(index));  // <- use `with` extractor
/// }
/// ```
///
/// The `Json` type allows you to respond with well-formed JSON data: simply
/// return a value of type Json<T> where T is the type of a structure
/// to serialize into *JSON*. The type `T` must implement the `Serialize`
/// trait from *serde*.
///
/// ```rust
/// # extern crate actix_web;
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// #
/// #[derive(Serialize)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(req: HttpRequest) -> Result<Json<MyObj>> {
///     Ok(Json(MyObj {
///         name: req.match_info().query("name")?,
///     }))
/// }
/// # fn main() {}
/// ```
pub struct Json<T>(pub T);

impl<T> Json<T> {
    /// Deconstruct to an inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> fmt::Debug for Json<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Json: {:?}", self.0)
    }
}

impl<T> fmt::Display for Json<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<T: Serialize> Responder for Json<T> {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to<S>(self, req: &HttpRequest<S>) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&self.0)?;

        Ok(req
            .build_response(StatusCode::OK)
            .content_type("application/json")
            .body(body))
    }
}

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned + 'static,
    S: 'static,
{
    type Config = JsonConfig<S>;
    type Result = Box<Future<Item = Self, Error = Error>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        let req2 = req.clone();
        let err = Rc::clone(&cfg.ehandler);
        Box::new(
            JsonBody::new(req)
                .limit(cfg.limit)
                .map_err(move |e| (*err)(e, &req2))
                .map(Json),
        )
    }
}

/// Json extractor configuration
///
/// ```rust
/// # extern crate actix_web;
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{error, http, App, HttpResponse, Json, Result};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body, max payload size is 4kb
/// fn index(info: Json<Info>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource("/index.html", |r| {
///         r.method(http::Method::POST)
///               .with_config(index, |cfg| {
///                   cfg.0.limit(4096)   // <- change json extractor configuration
///                      .error_handler(|err, req| {  // <- create custom error response
///                          error::InternalError::from_response(
///                              err, HttpResponse::Conflict().finish()).into()
///                          });
///               })
///     });
/// }
/// ```
pub struct JsonConfig<S> {
    limit: usize,
    ehandler: Rc<Fn(JsonPayloadError, &HttpRequest<S>) -> Error>,
}

impl<S> JsonConfig<S> {
    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(&mut self, limit: usize) -> &mut Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler
    pub fn error_handler<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(JsonPayloadError, &HttpRequest<S>) -> Error + 'static,
    {
        self.ehandler = Rc::new(f);
        self
    }
}

impl<S> Default for JsonConfig<S> {
    fn default() -> Self {
        JsonConfig {
            limit: 262_144,
            ehandler: Rc::new(|e, _| e.into()),
        }
    }
}

/// Request payload json parser that resolves to a deserialized `T` value.
///
/// Returns error:
///
/// * content type is not `application/json`
/// * content length is greater than 256k
///
/// # Server example
///
/// ```rust
/// # extern crate actix_web;
/// # extern crate futures;
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::{AsyncResponder, Error, HttpMessage, HttpRequest, HttpResponse};
/// use futures::future::Future;
///
/// #[derive(Deserialize, Debug)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(mut req: HttpRequest) -> Box<Future<Item = HttpResponse, Error = Error>> {
///     req.json()                   // <- get JsonBody future
///        .from_err()
///        .and_then(|val: MyObj| {  // <- deserialized value
///            println!("==== BODY ==== {:?}", val);
///            Ok(HttpResponse::Ok().into())
///        }).responder()
/// }
/// # fn main() {}
/// ```
pub struct JsonBody<T: HttpMessage, U: DeserializeOwned> {
    limit: usize,
    length: Option<usize>,
    stream: Option<T::Stream>,
    err: Option<JsonPayloadError>,
    fut: Option<Box<Future<Item = U, Error = JsonPayloadError>>>,
}

impl<T: HttpMessage, U: DeserializeOwned> JsonBody<T, U> {
    /// Create `JsonBody` for request.
    pub fn new(req: &T) -> Self {
        // check content-type
        let json = if let Ok(Some(mime)) = req.mime_type() {
            mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON)
        } else {
            false
        };
        if !json {
            return JsonBody {
                limit: 262_144,
                length: None,
                stream: None,
                fut: None,
                err: Some(JsonPayloadError::ContentType),
            };
        }

        let mut len = None;
        if let Some(l) = req.headers().get(CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                }
            }
        }

        JsonBody {
            limit: 262_144,
            length: len,
            stream: Some(req.payload()),
            fut: None,
            err: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl<T: HttpMessage + 'static, U: DeserializeOwned + 'static> Future for JsonBody<T, U> {
    type Item = U;
    type Error = JsonPayloadError;

    fn poll(&mut self) -> Poll<U, JsonPayloadError> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        let limit = self.limit;
        if let Some(len) = self.length.take() {
            if len > limit {
                return Err(JsonPayloadError::Overflow);
            }
        }

        let fut = self
            .stream
            .take()
            .expect("JsonBody could not be used second time")
            .from_err()
            .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                if (body.len() + chunk.len()) > limit {
                    Err(JsonPayloadError::Overflow)
                } else {
                    body.extend_from_slice(&chunk);
                    Ok(body)
                }
            }).and_then(|body| Ok(serde_json::from_slice::<U>(&body)?));
        self.fut = Some(Box::new(fut));
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::Async;
    use http::header;

    use handler::Handler;
    use test::TestRequest;
    use with::With;

    impl PartialEq for JsonPayloadError {
        fn eq(&self, other: &JsonPayloadError) -> bool {
            match *self {
                JsonPayloadError::Overflow => match *other {
                    JsonPayloadError::Overflow => true,
                    _ => false,
                },
                JsonPayloadError::ContentType => match *other {
                    JsonPayloadError::ContentType => true,
                    _ => false,
                },
                _ => false,
            }
        }
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    #[test]
    fn test_json() {
        let json = Json(MyObject {
            name: "test".to_owned(),
        });
        let resp = json.respond_to(&TestRequest::default().finish()).unwrap();
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_json_body() {
        let req = TestRequest::default().finish();
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let req = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            ).finish();
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let req = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            ).header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10000"),
            ).finish();
        let mut json = req.json::<MyObject>().limit(100);
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::Overflow);

        let req = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            ).header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ).set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .finish();

        let mut json = req.json::<MyObject>();
        assert_eq!(
            json.poll().ok().unwrap(),
            Async::Ready(MyObject {
                name: "test".to_owned()
            })
        );
    }

    #[test]
    fn test_with_json() {
        let mut cfg = JsonConfig::default();
        cfg.limit(4096);
        let handler = With::new(|data: Json<MyObject>| data, cfg);

        let req = TestRequest::default().finish();
        assert!(handler.handle(&req).as_err().is_some());

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        ).header(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("16"),
        ).set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
        .finish();
        assert!(handler.handle(&req).as_err().is_none())
    }
}

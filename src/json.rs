use std::fmt;
use std::ops::{Deref, DerefMut};
use bytes::{Bytes, BytesMut};
use futures::{Poll, Future, Stream};
use http::header::CONTENT_LENGTH;

use mime;
use serde_json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use error::{Error, JsonPayloadError, PayloadError};
use handler::{Responder, FromRequest};
use http::StatusCode;
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Json helper
///
/// Json can be used for two different purpose. First is for json response generation
/// and second is for extracting typed information from request's payload.
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

impl<T> fmt::Debug for Json<T> where T: fmt::Debug {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Json: {:?}", self.0)
    }
}

impl<T> fmt::Display for Json<T> where T: fmt::Display {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

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
///     Ok(Json(MyObj{name: req.match_info().query("name")?}))
/// }
/// # fn main() {}
/// ```
impl<T: Serialize> Responder for Json<T> {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&self.0)?;

        Ok(req.build_response(StatusCode::OK)
           .content_type("application/json")
           .body(body))
    }
}

/// To extract typed information from request's body, the type `T` must implement the
/// `Deserialize` trait from *serde*.
///
/// [**JsonConfig**](dev/struct.JsonConfig.html) allows to configure extraction process.
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
impl<T, S> FromRequest<S> for Json<T>
    where T: DeserializeOwned + 'static, S: 'static
{
    type Config = JsonConfig;
    type Result = Box<Future<Item=Self, Error=Error>>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        Box::new(
            JsonBody::new(req.clone())
                .limit(cfg.limit)
                .from_err()
                .map(Json))
    }
}

/// Json extractor configuration
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
/// /// deserialize `Info` from request's body, max payload size is 4kb
/// fn index(info: Json<Info>) -> Result<String> {
///     Ok(format!("Welcome {}!", info.username))
/// }
///
/// fn main() {
///     let app = App::new().resource(
///        "/index.html", |r| {
///            r.method(http::Method::POST)
///               .with(index)
///               .limit(4096);} // <- change json extractor configuration
///     );
/// }
/// ```
pub struct JsonConfig {
    limit: usize,
}

impl JsonConfig {

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(&mut self, limit: usize) -> &mut Self {
        self.limit = limit;
        self
    }
}

impl Default for JsonConfig {
    fn default() -> Self {
        JsonConfig{limit: 262_144}
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
/// use futures::future::Future;
/// use actix_web::{AsyncResponder, HttpRequest, HttpResponse, HttpMessage, Error};
///
/// #[derive(Deserialize, Debug)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(mut req: HttpRequest) -> Box<Future<Item=HttpResponse, Error=Error>> {
///     req.json()                   // <- get JsonBody future
///        .from_err()
///        .and_then(|val: MyObj| {  // <- deserialized value
///            println!("==== BODY ==== {:?}", val);
///            Ok(HttpResponse::Ok().into())
///        }).responder()
/// }
/// # fn main() {}
/// ```
pub struct JsonBody<T, U: DeserializeOwned>{
    limit: usize,
    req: Option<T>,
    fut: Option<Box<Future<Item=U, Error=JsonPayloadError>>>,
}

impl<T, U: DeserializeOwned> JsonBody<T, U> {

    /// Create `JsonBody` for request.
    pub fn new(req: T) -> Self {
        JsonBody{
            limit: 262_144,
            req: Some(req),
            fut: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl<T, U: DeserializeOwned + 'static> Future for JsonBody<T, U>
    where T: HttpMessage + Stream<Item=Bytes, Error=PayloadError> + 'static
{
    type Item = U;
    type Error = JsonPayloadError;

    fn poll(&mut self) -> Poll<U, JsonPayloadError> {
        if let Some(req) = self.req.take() {
            if let Some(len) = req.headers().get(CONTENT_LENGTH) {
                if let Ok(s) = len.to_str() {
                    if let Ok(len) = s.parse::<usize>() {
                        if len > self.limit {
                            return Err(JsonPayloadError::Overflow);
                        }
                    } else {
                        return Err(JsonPayloadError::Overflow);
                    }
                }
            }
            // check content-type

            let json = if let Ok(Some(mime)) = req.mime_type() {
                mime.subtype() == mime::JSON || mime.suffix() == Some(mime::JSON)
            } else {
                false
            };
            if !json {
                return Err(JsonPayloadError::ContentType)
            }

            let limit = self.limit;
            let fut = req.from_err()
                .fold(BytesMut::new(), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(JsonPayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .and_then(|body| Ok(serde_json::from_slice::<U>(&body)?));
            self.fut = Some(Box::new(fut));
        }

        self.fut.as_mut().expect("JsonBody could not be used second time").poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http::header;
    use futures::Async;

    use with::{With, ExtractorConfig};
    use handler::Handler;

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
        let json = Json(MyObject{name: "test".to_owned()});
        let resp = json.respond_to(HttpRequest::default()).unwrap();
        assert_eq!(resp.headers().get(header::CONTENT_TYPE).unwrap(), "application/json");
    }

    #[test]
    fn test_json_body() {
        let req = HttpRequest::default();
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let mut req = HttpRequest::default();
        req.headers_mut().insert(header::CONTENT_TYPE,
                                 header::HeaderValue::from_static("application/text"));
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let mut req = HttpRequest::default();
        req.headers_mut().insert(header::CONTENT_TYPE,
                                 header::HeaderValue::from_static("application/json"));
        req.headers_mut().insert(header::CONTENT_LENGTH,
                                 header::HeaderValue::from_static("10000"));
        let mut json = req.json::<MyObject>().limit(100);
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::Overflow);

        let mut req = HttpRequest::default();
        req.headers_mut().insert(header::CONTENT_TYPE,
                                 header::HeaderValue::from_static("application/json"));
        req.headers_mut().insert(header::CONTENT_LENGTH,
                                 header::HeaderValue::from_static("16"));
        req.payload_mut().unread_data(Bytes::from_static(b"{\"name\": \"test\"}"));
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().ok().unwrap(),
                   Async::Ready(MyObject{name: "test".to_owned()}));
    }

    #[test]
    fn test_with_json() {
        let mut cfg = ExtractorConfig::<_, Json<MyObject>>::default();
        cfg.limit(4096);
        let mut handler = With::new(|data: Json<MyObject>| {data}, cfg);

        let req = HttpRequest::default();
        let err = handler.handle(req).as_response().unwrap().error().is_some();
        assert!(err);

        let mut req = HttpRequest::default();
        req.headers_mut().insert(header::CONTENT_TYPE,
                                 header::HeaderValue::from_static("application/json"));
        req.headers_mut().insert(header::CONTENT_LENGTH,
                                 header::HeaderValue::from_static("16"));
        req.payload_mut().unread_data(Bytes::from_static(b"{\"name\": \"test\"}"));
        let ok = handler.handle(req).as_response().unwrap().error().is_none();
        assert!(ok)
    }
}

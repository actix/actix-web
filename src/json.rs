use bytes::{Bytes, BytesMut};
use futures::{Poll, Future, Stream};
use http::header::CONTENT_LENGTH;

use serde_json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use error::{Error, JsonPayloadError, PayloadError};
use handler::Responder;
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

/// Json response helper
///
/// The `Json` type allows you to respond with well-formed JSON data: simply return a value of
/// type Json<T> where T is the type of a structure to serialize into *JSON*. The
/// type `T` must implement the `Serialize` trait from *serde*.
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
pub struct Json<T: Serialize> (pub T);

impl<T: Serialize> Responder for Json<T> {
    type Item = HttpResponse;
    type Error = Error;

    fn respond_to(self, _: HttpRequest) -> Result<HttpResponse, Error> {
        let body = serde_json::to_string(&self.0)?;

        Ok(HttpResponse::Ok()
           .content_type("application/json")
           .body(body)?)
    }
}

/// Request payload json parser that resolves to a deserialized `T` value.
///
/// Returns error:
///
/// * content type is not `application/json`
/// * content length is greater than 256k
///
///
/// # Server example
///
/// ```rust
/// # extern crate actix_web;
/// # extern crate futures;
/// # #[macro_use] extern crate serde_derive;
/// use actix_web::*;
/// use futures::future::Future;
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
///            Ok(httpcodes::HTTPOk.into())
///        }).responder()
/// }
/// # fn main() {}
/// ```
pub struct JsonBody<T, U: DeserializeOwned>{
    limit: usize,
    ct: &'static str,
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
            ct: "application/json",
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set allowed content type.
    ///
    /// By default *application/json* content type is used. Set content type
    /// to empty string if you want to disable content type check.
    pub fn content_type(mut self, ct: &'static str) -> Self {
        self.ct = ct;
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
            if !self.ct.is_empty() && req.content_type() != self.ct {
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
                                 header::HeaderValue::from_static("application/json"));
        let mut json = req.json::<MyObject>().content_type("text/json");
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
        assert_eq!(json.poll().ok().unwrap(), Async::Ready(MyObject{name: "test".to_owned()}));
    }
}

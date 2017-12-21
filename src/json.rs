use bytes::BytesMut;
use futures::{Poll, Future, Stream};
use http::header::CONTENT_LENGTH;

use serde_json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use error::{Error, JsonPayloadError};
use handler::Responder;
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
///            Ok(httpcodes::HTTPOk.response())
///        }).responder()
/// }
/// # fn main() {}
/// ```
pub struct JsonBody<S, T: DeserializeOwned>{
    limit: usize,
    ct: &'static str,
    req: Option<HttpRequest<S>>,
    fut: Option<Box<Future<Item=T, Error=JsonPayloadError>>>,
}

impl<S, T: DeserializeOwned> JsonBody<S, T> {

    /// Create `JsonBody` for request.
    pub fn from_request(req: &mut HttpRequest<S>) -> Self {
        JsonBody{
            limit: 262_144,
            req: Some(req.clone()),
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

impl<S, T: DeserializeOwned + 'static> Future for JsonBody<S, T> {
    type Item = T;
    type Error = JsonPayloadError;

    fn poll(&mut self) -> Poll<T, JsonPayloadError> {
        if let Some(mut req) = self.req.take() {
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
            let fut = req.payload_mut().readany()
                .from_err()
                .fold(BytesMut::new(), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(JsonPayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .and_then(|body| Ok(serde_json::from_slice::<T>(&body)?));
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
        let mut req = HttpRequest::default();
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let mut json = req.json::<MyObject>().content_type("text/json");
        req.headers_mut().insert(header::CONTENT_TYPE,
                                 header::HeaderValue::from_static("application/json"));
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::ContentType);

        let mut json = req.json::<MyObject>().limit(100);
        req.headers_mut().insert(header::CONTENT_TYPE,
                                 header::HeaderValue::from_static("application/json"));
        req.headers_mut().insert(header::CONTENT_LENGTH,
                                 header::HeaderValue::from_static("10000"));
        assert_eq!(json.poll().err().unwrap(), JsonPayloadError::Overflow);

        req.headers_mut().insert(header::CONTENT_LENGTH,
                                 header::HeaderValue::from_static("16"));
        req.payload_mut().unread_data(Bytes::from_static(b"{\"name\": \"test\"}"));
        let mut json = req.json::<MyObject>();
        assert_eq!(json.poll().ok().unwrap(), Async::Ready(MyObject{name: "test".to_owned()}));
    }

}

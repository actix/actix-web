//! Json extractor/responder

use std::sync::Arc;
use std::{fmt, ops};

use bytes::BytesMut;
use futures::{Future, Poll, Stream};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json;

use actix_http::http::{header::CONTENT_LENGTH, StatusCode};
use actix_http::{HttpMessage, Payload, Response};

use crate::dev::Decompress;
use crate::error::{Error, JsonPayloadError};
use crate::extract::FromRequest;
use crate::request::HttpRequest;
use crate::responder::Responder;

/// Json helper
///
/// Json can be used for two different purpose. First is for json response
/// generation and second is for extracting typed information from request's
/// payload.
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**JsonConfig**](struct.JsonConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// fn index(info: web::Json<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().service(
///        web::resource("/index.html").route(
///            web::post().to(index))
///     );
/// }
/// ```
///
/// The `Json` type allows you to respond with well-formed JSON data: simply
/// return a value of type Json<T> where T is the type of a structure
/// to serialize into *JSON*. The type `T` must implement the `Serialize`
/// trait from *serde*.
///
/// ```rust
/// # #[macro_use] extern crate serde_derive;
/// # use actix_web::*;
/// #
/// #[derive(Serialize)]
/// struct MyObj {
///     name: String,
/// }
///
/// fn index(req: HttpRequest) -> Result<web::Json<MyObj>> {
///     Ok(web::Json(MyObj {
///         name: req.match_info().get("name").unwrap().to_string(),
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

impl<T> ops::Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> ops::DerefMut for Json<T> {
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
    type Error = Error;
    type Future = Result<Response, Error>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let body = match serde_json::to_string(&self.0) {
            Ok(body) => body,
            Err(e) => return Err(e.into()),
        };

        Ok(Response::build(StatusCode::OK)
            .content_type("application/json")
            .body(body))
    }
}

/// Json extractor. Allow to extract typed information from request's
/// payload.
///
/// To extract typed information from request's body, the type `T` must
/// implement the `Deserialize` trait from *serde*.
///
/// [**JsonConfig**](struct.JsonConfig.html) allows to configure extraction
/// process.
///
/// ## Example
///
/// ```rust
/// #[macro_use] extern crate serde_derive;
/// use actix_web::{web, App};
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// fn index(info: web::Json<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").route(
///            web::post().to(index))
///     );
/// }
/// ```
impl<T> FromRequest for Json<T>
where
    T: DeserializeOwned + 'static,
{
    type Error = Error;
    type Future = Box<dyn Future<Item = Self, Error = Error>>;
    type Config = JsonConfig;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let req2 = req.clone();
        let (limit, err, ctype) = req
            .app_data::<Self::Config>()
            .map(|c| (c.limit, c.ehandler.clone(), c.content_type.clone()))
            .unwrap_or((32768, None, None));

        Box::new(
            JsonBody::new(req, payload, ctype)
                .limit(limit)
                .map_err(move |e| {
                    log::debug!(
                        "Failed to deserialize Json from payload. \
                         Request path: {}",
                        req2.path()
                    );
                    if let Some(err) = err {
                        (*err)(e, &req2)
                    } else {
                        e.into()
                    }
                })
                .map(Json),
        )
    }
}

/// Json extractor configuration
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
/// /// deserialize `Info` from request's body, max payload size is 4kb
/// fn index(info: web::Json<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").data(
///             // change json extractor configuration
///             web::Json::<Info>::configure(|cfg| {
///                 cfg.limit(4096)
///                    .content_type(|mime| {  // <- accept text/plain content type
///                         mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
///                    })
///                    .error_handler(|err, req| {  // <- create custom error response
///                         error::InternalError::from_response(
///                             err, HttpResponse::Conflict().finish()).into()
///                    })
///             }))
///             .route(web::post().to(index))
///     );
/// }
/// ```
#[derive(Clone)]
pub struct JsonConfig {
    limit: usize,
    ehandler: Option<Arc<dyn Fn(JsonPayloadError, &HttpRequest) -> Error + Send + Sync>>,
    content_type: Option<Arc<dyn Fn(mime::Mime) -> bool + Send + Sync>>,
}

impl JsonConfig {
    /// Change max size of payload. By default max size is 32Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(JsonPayloadError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.ehandler = Some(Arc::new(f));
        self
    }

    /// Set predicate for allowed content types
    pub fn content_type<F>(mut self, predicate: F) -> Self
    where
        F: Fn(mime::Mime) -> bool + Send + Sync + 'static,
    {
        self.content_type = Some(Arc::new(predicate));
        self
    }
}

impl Default for JsonConfig {
    fn default() -> Self {
        JsonConfig {
            limit: 32768,
            ehandler: None,
            content_type: None,
        }
    }
}

/// Request's payload json parser, it resolves to a deserialized `T` value.
/// This future could be used with `ServiceRequest` and `ServiceFromRequest`.
///
/// Returns error:
///
/// * content type is not `application/json`
///   (unless specified in [`JsonConfig`](struct.JsonConfig.html))
/// * content length is greater than 256k
pub struct JsonBody<U> {
    limit: usize,
    length: Option<usize>,
    stream: Option<Decompress<Payload>>,
    err: Option<JsonPayloadError>,
    fut: Option<Box<dyn Future<Item = U, Error = JsonPayloadError>>>,
}

impl<U> JsonBody<U>
where
    U: DeserializeOwned + 'static,
{
    /// Create `JsonBody` for request.
    pub fn new(
        req: &HttpRequest,
        payload: &mut Payload,
        ctype: Option<Arc<dyn Fn(mime::Mime) -> bool + Send + Sync>>,
    ) -> Self {
        // check content-type
        let json = if let Ok(Some(mime)) = req.mime_type() {
            mime.subtype() == mime::JSON
                || mime.suffix() == Some(mime::JSON)
                || ctype.as_ref().map_or(false, |predicate| predicate(mime))
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

        let len = req
            .headers()
            .get(&CONTENT_LENGTH)
            .and_then(|l| l.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok());
        let payload = Decompress::from_headers(payload.take(), req.headers());

        JsonBody {
            limit: 262_144,
            length: len,
            stream: Some(payload),
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

impl<U> Future for JsonBody<U>
where
    U: DeserializeOwned + 'static,
{
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
            .unwrap()
            .from_err()
            .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                if (body.len() + chunk.len()) > limit {
                    Err(JsonPayloadError::Overflow)
                } else {
                    body.extend_from_slice(&chunk);
                    Ok(body)
                }
            })
            .and_then(|body| Ok(serde_json::from_slice::<U>(&body)?));
        self.fut = Some(Box::new(fut));
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde_derive::{Deserialize, Serialize};

    use super::*;
    use crate::error::InternalError;
    use crate::http::header;
    use crate::test::{block_on, TestRequest};
    use crate::HttpResponse;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    fn json_eq(err: JsonPayloadError, other: JsonPayloadError) -> bool {
        match err {
            JsonPayloadError::Overflow => match other {
                JsonPayloadError::Overflow => true,
                _ => false,
            },
            JsonPayloadError::ContentType => match other {
                JsonPayloadError::ContentType => true,
                _ => false,
            },
            _ => false,
        }
    }

    #[test]
    fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let j = Json(MyObject {
            name: "test".to_string(),
        });
        let resp = j.respond_to(&req).unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            header::HeaderValue::from_static("application/json")
        );

        use crate::responder::tests::BodyTest;
        assert_eq!(resp.body().bin_ref(), b"{\"name\":\"test\"}");
    }

    #[test]
    fn test_custom_error_responder() {
        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            )
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .data(JsonConfig::default().limit(10).error_handler(|err, _| {
                let msg = MyObject {
                    name: "invalid request".to_string(),
                };
                let resp = HttpResponse::BadRequest()
                    .body(serde_json::to_string(&msg).unwrap());
                InternalError::from_response(err, resp).into()
            }))
            .to_http_parts();

        let s = block_on(Json::<MyObject>::from_request(&req, &mut pl));
        let mut resp = Response::from_error(s.err().unwrap().into());
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = block_on(resp.take_body().concat2()).unwrap();
        let msg: MyObject = serde_json::from_slice(&body).unwrap();
        assert_eq!(msg.name, "invalid request");
    }

    #[test]
    fn test_extract() {
        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            )
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .to_http_parts();

        let s = block_on(Json::<MyObject>::from_request(&req, &mut pl)).unwrap();
        assert_eq!(s.name, "test");
        assert_eq!(
            s.into_inner(),
            MyObject {
                name: "test".to_string()
            }
        );

        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            )
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .data(JsonConfig::default().limit(10))
            .to_http_parts();

        let s = block_on(Json::<MyObject>::from_request(&req, &mut pl));
        assert!(format!("{}", s.err().unwrap())
            .contains("Json payload size is bigger than allowed"));

        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            )
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .data(
                JsonConfig::default()
                    .limit(10)
                    .error_handler(|_, _| JsonPayloadError::ContentType.into()),
            )
            .to_http_parts();
        let s = block_on(Json::<MyObject>::from_request(&req, &mut pl));
        assert!(format!("{}", s.err().unwrap()).contains("Content type error"));
    }

    #[test]
    fn test_json_body() {
        let (req, mut pl) = TestRequest::default().to_http_parts();
        let json = block_on(JsonBody::<MyObject>::new(&req, &mut pl, None));
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            )
            .to_http_parts();
        let json = block_on(JsonBody::<MyObject>::new(&req, &mut pl, None));
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10000"),
            )
            .to_http_parts();

        let json = block_on(JsonBody::<MyObject>::new(&req, &mut pl, None).limit(100));
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::Overflow));

        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            )
            .header(
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            )
            .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
            .to_http_parts();

        let json = block_on(JsonBody::<MyObject>::new(&req, &mut pl, None));
        assert_eq!(
            json.ok().unwrap(),
            MyObject {
                name: "test".to_owned()
            }
        );
    }

    #[test]
    fn test_with_json_and_bad_content_type() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain"),
        )
        .header(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("16"),
        )
        .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
        .data(JsonConfig::default().limit(4096))
        .to_http_parts();

        let s = block_on(Json::<MyObject>::from_request(&req, &mut pl));
        assert!(s.is_err())
    }

    #[test]
    fn test_with_json_and_good_custom_content_type() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain"),
        )
        .header(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("16"),
        )
        .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
        .data(JsonConfig::default().content_type(|mime: mime::Mime| {
            mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
        }))
        .to_http_parts();

        let s = block_on(Json::<MyObject>::from_request(&req, &mut pl));
        assert!(s.is_ok())
    }

    #[test]
    fn test_with_json_and_bad_custom_content_type() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/html"),
        )
        .header(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("16"),
        )
        .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
        .data(JsonConfig::default().content_type(|mime: mime::Mime| {
            mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
        }))
        .to_http_parts();

        let s = block_on(Json::<MyObject>::from_request(&req, &mut pl));
        assert!(s.is_err())
    }
}

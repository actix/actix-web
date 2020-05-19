//! Json extractor/responder

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{fmt, ops};

use bytes::BytesMut;
use futures_util::future::{err, ok, FutureExt, LocalBoxFuture, Ready};
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde::Serialize;

use actix_http::http::{header::CONTENT_LENGTH, StatusCode};
use actix_http::{HttpMessage, Payload, Response};

#[cfg(feature = "compress")]
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
/// use actix_web::{web, App};
/// use serde_derive::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// async fn index(info: web::Json<Info>) -> String {
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
/// use actix_web::*;
/// use serde_derive::Serialize;
///
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Json: {:?}", self.0)
    }
}

impl<T> fmt::Display for Json<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<T: Serialize> Responder for Json<T> {
    type Error = Error;
    type Future = Ready<Result<Response, Error>>;

    fn respond_to(self, _: &HttpRequest) -> Self::Future {
        let body = match serde_json::to_string(&self.0) {
            Ok(body) => body,
            Err(e) => return err(e.into()),
        };

        ok(Response::build(StatusCode::OK)
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
/// use actix_web::{web, App};
/// use serde_derive::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// async fn index(info: web::Json<Info>) -> String {
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
    type Future = LocalBoxFuture<'static, Result<Self, Error>>;
    type Config = JsonConfig;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let req2 = req.clone();
        let (limit, err, ctype) = req
            .app_data::<Self::Config>()
            .map(|c| (c.limit, c.ehandler.clone(), c.content_type.clone()))
            .unwrap_or((32768, None, None));

        JsonBody::new(req, payload, ctype)
            .limit(limit)
            .map(move |res| match res {
                Err(e) => {
                    log::error!(
                        "Failed to deserialize Json from payload. \
                         Request path: {}",
                        req2.path()
                    );
                    if let Some(err) = err {
                        Err((*err)(e, &req2))
                    } else {
                        Err(e.into())
                    }
                }
                Ok(data) => Ok(Json(data)),
            })
            .boxed_local()
    }
}

/// Json extractor configuration
///
/// # Examples
///
/// ```rust
/// use actix_web::{error, web, App, FromRequest, HttpRequest, HttpResponse};
/// use serde_derive::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body, max payload size is 4kb
/// async fn index(info: web::Json<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
///
/// /// Return either a 400 or 415, and include the error message from serde
/// /// in the response body
/// fn json_error_handler(err: error::JsonPayloadError, _req: &HttpRequest) -> error::Error {
///     let detail = err.to_string();
///     let response = match &err {
///         error::JsonPayloadError::ContentType => {
///             HttpResponse::UnsupportedMediaType().content_type("text/plain").body(detail)
///         }
///         _ => HttpResponse::BadRequest().content_type("text/plain").body(detail),
///     };
///     error::InternalError::from_response(err, response).into()
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html")
///             .app_data(
///                 // change json extractor configuration
///                 web::Json::<Info>::configure(|cfg| {
///                     cfg.limit(4096)
///                        .content_type(|mime| {  // <- accept text/plain content type
///                            mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
///                        })
///                        .error_handler(json_error_handler)  // Use our custom error response
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
    #[cfg(feature = "compress")]
    stream: Option<Decompress<Payload>>,
    #[cfg(not(feature = "compress"))]
    stream: Option<Payload>,
    err: Option<JsonPayloadError>,
    fut: Option<LocalBoxFuture<'static, Result<U, JsonPayloadError>>>,
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

        #[cfg(feature = "compress")]
        let payload = Decompress::from_headers(payload.take(), req.headers());
        #[cfg(not(feature = "compress"))]
        let payload = payload.take();

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
    type Output = Result<U, JsonPayloadError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(ref mut fut) = self.fut {
            return Pin::new(fut).poll(cx);
        }

        if let Some(err) = self.err.take() {
            return Poll::Ready(Err(err));
        }

        let limit = self.limit;
        if let Some(len) = self.length.take() {
            if len > limit {
                return Poll::Ready(Err(JsonPayloadError::Overflow));
            }
        }
        let mut stream = self.stream.take().unwrap();

        self.fut = Some(
            async move {
                let mut body = BytesMut::with_capacity(8192);

                while let Some(item) = stream.next().await {
                    let chunk = item?;
                    if (body.len() + chunk.len()) > limit {
                        return Err(JsonPayloadError::Overflow);
                    } else {
                        body.extend_from_slice(&chunk);
                    }
                }
                Ok(serde_json::from_slice::<U>(&body)?)
            }
            .boxed_local(),
        );

        self.poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde_derive::{Deserialize, Serialize};

    use super::*;
    use crate::error::InternalError;
    use crate::http::header;
    use crate::test::{load_stream, TestRequest};
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

    #[actix_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let j = Json(MyObject {
            name: "test".to_string(),
        });
        let resp = j.respond_to(&req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            header::HeaderValue::from_static("application/json")
        );

        use crate::responder::tests::BodyTest;
        assert_eq!(resp.body().bin_ref(), b"{\"name\":\"test\"}");
    }

    #[actix_rt::test]
    async fn test_custom_error_responder() {
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
            .app_data(JsonConfig::default().limit(10).error_handler(|err, _| {
                let msg = MyObject {
                    name: "invalid request".to_string(),
                };
                let resp = HttpResponse::BadRequest()
                    .body(serde_json::to_string(&msg).unwrap());
                InternalError::from_response(err, resp).into()
            }))
            .to_http_parts();

        let s = Json::<MyObject>::from_request(&req, &mut pl).await;
        let mut resp = Response::from_error(s.err().unwrap().into());
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = load_stream(resp.take_body()).await.unwrap();
        let msg: MyObject = serde_json::from_slice(&body).unwrap();
        assert_eq!(msg.name, "invalid request");
    }

    #[actix_rt::test]
    async fn test_extract() {
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

        let s = Json::<MyObject>::from_request(&req, &mut pl).await.unwrap();
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
            .app_data(JsonConfig::default().limit(10))
            .to_http_parts();

        let s = Json::<MyObject>::from_request(&req, &mut pl).await;
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
            .app_data(
                JsonConfig::default()
                    .limit(10)
                    .error_handler(|_, _| JsonPayloadError::ContentType.into()),
            )
            .to_http_parts();
        let s = Json::<MyObject>::from_request(&req, &mut pl).await;
        assert!(format!("{}", s.err().unwrap()).contains("Content type error"));
    }

    #[actix_rt::test]
    async fn test_json_body() {
        let (req, mut pl) = TestRequest::default().to_http_parts();
        let json = JsonBody::<MyObject>::new(&req, &mut pl, None).await;
        assert!(json_eq(json.err().unwrap(), JsonPayloadError::ContentType));

        let (req, mut pl) = TestRequest::default()
            .header(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            )
            .to_http_parts();
        let json = JsonBody::<MyObject>::new(&req, &mut pl, None).await;
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

        let json = JsonBody::<MyObject>::new(&req, &mut pl, None)
            .limit(100)
            .await;
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

        let json = JsonBody::<MyObject>::new(&req, &mut pl, None).await;
        assert_eq!(
            json.ok().unwrap(),
            MyObject {
                name: "test".to_owned()
            }
        );
    }

    #[actix_rt::test]
    async fn test_with_json_and_bad_content_type() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain"),
        )
        .header(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("16"),
        )
        .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
        .app_data(JsonConfig::default().limit(4096))
        .to_http_parts();

        let s = Json::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_err())
    }

    #[actix_rt::test]
    async fn test_with_json_and_good_custom_content_type() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain"),
        )
        .header(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("16"),
        )
        .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
        .app_data(JsonConfig::default().content_type(|mime: mime::Mime| {
            mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
        }))
        .to_http_parts();

        let s = Json::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_ok())
    }

    #[actix_rt::test]
    async fn test_with_json_and_bad_custom_content_type() {
        let (req, mut pl) = TestRequest::with_header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/html"),
        )
        .header(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("16"),
        )
        .set_payload(Bytes::from_static(b"{\"name\": \"test\"}"))
        .app_data(JsonConfig::default().content_type(|mime: mime::Mime| {
            mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
        }))
        .to_http_parts();

        let s = Json::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_err())
    }
}

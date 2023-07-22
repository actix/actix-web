//! For Bincode helper documentation, see [`Bincode`].
#![cfg(feature = "bincode")]

use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    ops,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use actix_http::Payload;
use bytes::BytesMut;
use futures_core::{ready, Stream as _};
use serde::{de::DeserializeOwned, Serialize};

#[cfg(feature = "__compress")]
use crate::dev::Decompress;
use crate::{
    body::EitherBody,
    error::{BincodePayloadError, Error},
    extract::FromRequest,
    http::header::{ContentLength, Header as _},
    request::HttpRequest,
    web, HttpMessage, HttpResponse, Responder,
};

/// Bincode extractor and responder.
///
/// `Bincode` has two uses: Bincode responses, and extracting typed data from Bincode request payloads.
///
/// # Extractor
/// To extract typed data from a request body, the inner type `T` must implement the
/// [`serde::Deserialize`] trait.
///
/// Use [`BincodeConfig`] to configure extraction options.
///
/// ```
/// use actix_web::{post, web, App};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Info {
///     username: String,
/// }
///
/// /// deserialize `Info` from request's body
/// #[post("/")]
/// async fn index(info: web::Bincode<Info>) -> String {
///     format!("Welcome {}!", info.username)
/// }
/// ```
///
/// # Responder
/// The `Bincode` type  Bincode formatted responses. A handler may return a value of type
/// `Bincode<T>` where `T` is the type of a structure to serialize into Bincode. The type `T` must
/// implement [`serde::Serialize`].
///
/// ```
/// use actix_web::{post, web, HttpRequest};
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct Info {
///     name: String,
/// }
///
/// #[post("/{name}")]
/// async fn index(req: HttpRequest) -> web::Bincode<Info> {
///     web::Bincode(Info {
///         name: req.match_info().get("name").unwrap().to_owned(),
///     })
/// }
/// ```
#[derive(Debug)]
pub struct Bincode<T>(pub T);

impl<T> Bincode<T> {
    /// Unwrap into inner `T` value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> ops::Deref for Bincode<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> ops::DerefMut for Bincode<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: fmt::Display> fmt::Display for Bincode<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<T: Serialize> Serialize for Bincode<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

/// Creates response with OK status code, correct content type header, and serialized Bincode payload.
///
/// If serialization failed
impl<T: Serialize> Responder for Bincode<T> {
    type Body = EitherBody<Vec<u8>>;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<Self::Body> {
        match bincode::serialize(&self.0) {
            Ok(body) => match HttpResponse::Ok()
                .content_type(mime::APPLICATION_OCTET_STREAM)
                .message_body(body)
            {
                Ok(res) => res.map_into_left_body(),
                Err(err) => HttpResponse::from_error(err).map_into_right_body(),
            },

            Err(err) => {
                HttpResponse::from_error(BincodePayloadError::Serialize(err)).map_into_right_body()
            }
        }
    }
}

/// See [here](#extractor) for example of usage as an extractor.
impl<T: DeserializeOwned> FromRequest for Bincode<T> {
    type Error = Error;
    type Future = BincodeExtractFut<T>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let config = BincodeConfig::from_req(req);

        let limit = config.limit;
        let ctype_required = config.content_type_required;
        let ctype_fn = config.content_type.as_deref();
        let err_handler = config.err_handler.clone();

        BincodeExtractFut {
            req: Some(req.clone()),
            fut: BincodeBody::new(req, payload, ctype_fn, ctype_required).limit(limit),
            err_handler,
        }
    }
}

type BincodeErrorHandler =
    Option<Arc<dyn Fn(BincodePayloadError, &HttpRequest) -> Error + Send + Sync>>;

pub struct BincodeExtractFut<T> {
    req: Option<HttpRequest>,
    fut: BincodeBody<T>,
    err_handler: BincodeErrorHandler,
}

impl<T: DeserializeOwned> Future for BincodeExtractFut<T> {
    type Output = Result<Bincode<T>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        let res = ready!(Pin::new(&mut this.fut).poll(cx));

        let res = match res {
            Err(err) => {
                let req = this.req.take().unwrap();
                log::debug!(
                    "Failed to deserialize Bincode from payload. \
                         Request path: {}",
                    req.path()
                );

                if let Some(err_handler) = this.err_handler.as_ref() {
                    Err((*err_handler)(err, &req))
                } else {
                    Err(err.into())
                }
            }
            Ok(data) => Ok(Bincode(data)),
        };

        Poll::Ready(res)
    }
}

/// `Bincode` extractor configuration.
///
/// # Examples
/// ```
/// use actix_web::{error, post, web, App, FromRequest, HttpResponse};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Info {
///     name: String,
/// }
///
/// // `Bincode` extraction is bound by custom `BincodeConfig` applied to App.
/// #[post("/")]
/// async fn index(info: web::Bincode<Info>) -> String {
///     format!("Welcome {}!", info.name)
/// }
///
/// // custom `Bincode` extractor configuration
/// let bincode_cfg = web::BincodeConfig::default()
///     // limit request payload size
///     .limit(4096)
///     // only accept text/plain content type
///     .content_type(|mime| mime == mime::TEXT_PLAIN)
///     // use custom error handler
///     .error_handler(|err, req| {
///         error::InternalError::from_response(err, HttpResponse::Conflict().into()).into()
///     });
///
/// App::new()
///     .app_data(bincode_cfg)
///     .service(index);
/// ```
#[derive(Clone)]
pub struct BincodeConfig {
    limit: usize,
    err_handler: BincodeErrorHandler,
    content_type: Option<Arc<dyn Fn(mime::Mime) -> bool + Send + Sync>>,
    content_type_required: bool,
}

impl BincodeConfig {
    /// Set maximum accepted payload size. By default this limit is 2MB.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set custom error handler.
    pub fn error_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(BincodePayloadError, &HttpRequest) -> Error + Send + Sync + 'static,
    {
        self.err_handler = Some(Arc::new(f));
        self
    }

    /// Set predicate for allowed content types.
    pub fn content_type<F>(mut self, predicate: F) -> Self
    where
        F: Fn(mime::Mime) -> bool + Send + Sync + 'static,
    {
        self.content_type = Some(Arc::new(predicate));
        self
    }

    /// Sets whether or not the request must have a `Content-Type` header to be parsed.
    pub fn content_type_required(mut self, content_type_required: bool) -> Self {
        self.content_type_required = content_type_required;
        self
    }

    /// Extract payload config from app data. Check both `T` and `Data<T>`, in that order, and fall
    /// back to the default payload config.
    fn from_req(req: &HttpRequest) -> &Self {
        req.app_data::<Self>()
            .or_else(|| req.app_data::<web::Data<Self>>().map(|d| d.as_ref()))
            .unwrap_or(&DEFAULT_CONFIG)
    }
}

const DEFAULT_LIMIT: usize = 2_097_152; // 2 mb

/// Allow shared refs used as default.
const DEFAULT_CONFIG: BincodeConfig = BincodeConfig {
    limit: DEFAULT_LIMIT,
    err_handler: None,
    content_type: None,
    content_type_required: true,
};

impl Default for BincodeConfig {
    fn default() -> Self {
        DEFAULT_CONFIG
    }
}

/// Future that resolves to some `T` when parsed from a Bincode payload.
///
/// Can deserialize any type `T` that implements [`Deserialize`][serde::Deserialize].
///
/// Returns error if:
/// - `Content-Type` is not `application/octet-stream` when `ctype_required` (passed to [`new`][Self::new])
///   is `true`.
/// - `Content-Length` is greater than [limit](BincodeBody::limit()).
/// - The payload, when consumed, is not valid Bincode.
pub enum BincodeBody<T> {
    Error(Option<BincodePayloadError>),
    Body {
        limit: usize,
        /// Length as reported by `Content-Length` header, if present.
        length: Option<usize>,
        #[cfg(feature = "__compress")]
        payload: Decompress<Payload>,
        #[cfg(not(feature = "__compress"))]
        payload: Payload,
        buf: BytesMut,
        _res: PhantomData<T>,
    },
}

impl<T> Unpin for BincodeBody<T> {}

impl<T: DeserializeOwned> BincodeBody<T> {
    /// Create a new future to decode a Bincode request payload.
    #[allow(clippy::borrow_interior_mutable_const)]
    pub fn new(
        req: &HttpRequest,
        payload: &mut Payload,
        ctype_fn: Option<&(dyn Fn(mime::Mime) -> bool + Send + Sync)>,
        ctype_required: bool,
    ) -> Self {
        // check content-type
        let can_parse_bincode = if let Ok(Some(mime)) = req.mime_type() {
            mime.subtype() == mime::OCTET_STREAM
                || mime.suffix() == Some(mime::OCTET_STREAM)
                || ctype_fn.map_or(false, |predicate| predicate(mime))
        } else {
            // if `ctype_required` is false, assume payload is
            // bincode even when content-type header is missing
            !ctype_required
        };

        if !can_parse_bincode {
            return BincodeBody::Error(Some(BincodePayloadError::ContentType));
        }

        let length = ContentLength::parse(req).ok().map(|x| x.0);

        // Notice the content-length is not checked against limit of bincode config here.
        // As the internal usage always call BincodeBody::limit after BincodeBody::new.
        // And limit check to return an error variant of BincodeBody happens there.

        let payload = {
            cfg_if::cfg_if! {
                if #[cfg(feature = "__compress")] {
                    Decompress::from_headers(payload.take(), req.headers())
                } else {
                    payload.take()
                }
            }
        };

        BincodeBody::Body {
            limit: DEFAULT_LIMIT,
            length,
            payload,
            buf: BytesMut::with_capacity(8192),
            _res: PhantomData,
        }
    }

    /// Set maximum accepted payload size. The default limit is 2MB.
    pub fn limit(self, limit: usize) -> Self {
        match self {
            BincodeBody::Body {
                length,
                payload,
                buf,
                ..
            } => {
                if let Some(len) = length {
                    if len > limit {
                        return BincodeBody::Error(Some(
                            BincodePayloadError::OverflowKnownLength { length: len, limit },
                        ));
                    }
                }

                BincodeBody::Body {
                    limit,
                    length,
                    payload,
                    buf,
                    _res: PhantomData,
                }
            }
            BincodeBody::Error(e) => BincodeBody::Error(e),
        }
    }
}

impl<T: DeserializeOwned> Future for BincodeBody<T> {
    type Output = Result<T, BincodePayloadError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this {
            BincodeBody::Body {
                limit,
                buf,
                payload,
                ..
            } => loop {
                let res = ready!(Pin::new(&mut *payload).poll_next(cx));
                match res {
                    Some(chunk) => {
                        let chunk = chunk?;
                        let buf_len = buf.len() + chunk.len();
                        if buf_len > *limit {
                            return Poll::Ready(Err(BincodePayloadError::Overflow {
                                limit: *limit,
                            }));
                        } else {
                            buf.extend_from_slice(&chunk);
                        }
                    }
                    None => {
                        let bincode = bincode::deserialize::<T>(buf)
                            .map_err(BincodePayloadError::Deserialize)?;
                        return Poll::Ready(Ok(bincode));
                    }
                }
            },
            BincodeBody::Error(e) => Poll::Ready(Err(e.take().unwrap())),
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{
        body,
        error::InternalError,
        http::{
            header::{self, CONTENT_LENGTH, CONTENT_TYPE},
            StatusCode,
        },
        test::{assert_body_eq, TestRequest},
    };

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct MyObject {
        name: String,
    }

    const BINCODE: &[u8] = b"\x04\0\0\0\0\0\0\0test";

    fn bincode_eq(err: BincodePayloadError, other: BincodePayloadError) -> bool {
        match err {
            BincodePayloadError::Overflow { .. } => {
                matches!(other, BincodePayloadError::Overflow { .. })
            }
            BincodePayloadError::OverflowKnownLength { .. } => {
                matches!(other, BincodePayloadError::OverflowKnownLength { .. })
            }
            BincodePayloadError::ContentType => matches!(other, BincodePayloadError::ContentType),
            _ => false,
        }
    }

    #[actix_rt::test]
    async fn test_responder() {
        let req = TestRequest::default().to_http_request();

        let j = Bincode(MyObject {
            name: "test".to_string(),
        });
        let res = j.respond_to(&req);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            header::HeaderValue::from_static("application/octet-stream")
        );
        assert_body_eq!(res, BINCODE);
    }

    #[actix_rt::test]
    async fn test_custom_error_responder() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(BincodeConfig::default().limit(10).error_handler(|err, _| {
                let msg = MyObject {
                    name: "invalid request".to_string(),
                };
                let resp = HttpResponse::BadRequest().body(bincode::serialize(&msg).unwrap());
                InternalError::from_response(err, resp).into()
            }))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        let resp = HttpResponse::from_error(s.unwrap_err());
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = body::to_bytes(resp.into_body()).await.unwrap();
        let msg: MyObject = bincode::deserialize(&body).unwrap();
        assert_eq!(msg.name, "invalid request");
    }

    #[actix_rt::test]
    async fn test_extract() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl)
            .await
            .unwrap();
        assert_eq!(s.name, "test");
        assert_eq!(
            s.into_inner(),
            MyObject {
                name: "test".to_string()
            }
        );

        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(BincodeConfig::default().limit(10))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        assert!(format!("{}", s.err().unwrap())
            .contains("Bincode payload (16 bytes) is larger than allowed (limit: 10 bytes)."));

        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(
                BincodeConfig::default()
                    .limit(10)
                    .error_handler(|_, _| BincodePayloadError::ContentType.into()),
            )
            .to_http_parts();
        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        assert!(format!("{}", s.err().unwrap()).contains("Content type error"));
    }

    #[actix_rt::test]
    async fn test_bincode_body() {
        let (req, mut pl) = TestRequest::default().to_http_parts();
        let bincode = BincodeBody::<MyObject>::new(&req, &mut pl, None, true).await;
        assert!(bincode_eq(
            bincode.err().unwrap(),
            BincodePayloadError::ContentType
        ));

        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/text"),
            ))
            .to_http_parts();
        let bincode = BincodeBody::<MyObject>::new(&req, &mut pl, None, true).await;
        assert!(bincode_eq(
            bincode.err().unwrap(),
            BincodePayloadError::ContentType
        ));

        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("10000"),
            ))
            .to_http_parts();

        let bincode = BincodeBody::<MyObject>::new(&req, &mut pl, None, true)
            .limit(100)
            .await;
        assert!(bincode_eq(
            bincode.err().unwrap(),
            BincodePayloadError::OverflowKnownLength {
                length: 10000,
                limit: 100
            }
        ));

        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            ))
            .set_payload(Bytes::from_static(&[0u8; 1000]))
            .to_http_parts();

        let bincode = BincodeBody::<MyObject>::new(&req, &mut pl, None, true)
            .limit(100)
            .await;

        assert!(bincode_eq(
            bincode.err().unwrap(),
            BincodePayloadError::Overflow { limit: 100 }
        ));

        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/octet-stream"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .to_http_parts();

        let bincode = BincodeBody::<MyObject>::new(&req, &mut pl, None, true).await;
        assert_eq!(
            bincode.ok().unwrap(),
            MyObject {
                name: "test".to_owned()
            }
        );
    }

    #[actix_rt::test]
    async fn test_with_bincode_and_bad_content_type() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/plain"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(BincodeConfig::default().limit(4096))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_err())
    }

    #[actix_rt::test]
    async fn test_with_bincode_and_good_custom_content_type() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/plain"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(BincodeConfig::default().content_type(|mime: mime::Mime| {
                mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
            }))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_ok())
    }

    #[actix_rt::test]
    async fn test_with_bincode_and_bad_custom_content_type() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/html"),
            ))
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(BincodeConfig::default().content_type(|mime: mime::Mime| {
                mime.type_() == mime::TEXT && mime.subtype() == mime::PLAIN
            }))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_err())
    }

    #[actix_rt::test]
    async fn test_bincode_with_no_content_type() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((
                header::CONTENT_LENGTH,
                header::HeaderValue::from_static("16"),
            ))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(BincodeConfig::default().content_type_required(false))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_ok())
    }

    #[actix_rt::test]
    async fn test_with_config_in_data_wrapper() {
        let (req, mut pl) = TestRequest::default()
            .insert_header((CONTENT_TYPE, mime::APPLICATION_OCTET_STREAM))
            .insert_header((CONTENT_LENGTH, 16))
            .set_payload(Bytes::from_static(BINCODE))
            .app_data(web::Data::new(BincodeConfig::default().limit(10)))
            .to_http_parts();

        let s = Bincode::<MyObject>::from_request(&req, &mut pl).await;
        assert!(s.is_err());

        let err_str = s.err().unwrap().to_string();
        assert!(err_str
            .contains("Bincode payload (16 bytes) is larger than allowed (limit: 10 bytes)."));
    }
}

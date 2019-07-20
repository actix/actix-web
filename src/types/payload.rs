//! Payload/Bytes/String extractors
use std::str;

use actix_http::error::{Error, ErrorBadRequest, PayloadError};
use actix_http::HttpMessage;
use bytes::{Bytes, BytesMut};
use encoding_rs::UTF_8;
use futures::future::{err, Either, FutureResult};
use futures::{Future, Poll, Stream};
use mime::Mime;

use crate::dev;
use crate::extract::FromRequest;
use crate::http::header;
use crate::request::HttpRequest;

/// Payload extractor returns request 's payload stream.
///
/// ## Example
///
/// ```rust
/// use futures::{Future, Stream};
/// use actix_web::{web, error, App, Error, HttpResponse};
///
/// /// extract binary data from request
/// fn index(body: web::Payload) -> impl Future<Item = HttpResponse, Error = Error>
/// {
///     body.map_err(Error::from)
///         .fold(web::BytesMut::new(), move |mut body, chunk| {
///             body.extend_from_slice(&chunk);
///             Ok::<_, Error>(body)
///          })
///          .and_then(|body| {
///              format!("Body {:?}!", body);
///              Ok(HttpResponse::Ok().finish())
///          })
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").route(
///             web::get().to_async(index))
///     );
/// }
/// ```
pub struct Payload(crate::dev::Payload);

impl Stream for Payload {
    type Item = Bytes;
    type Error = PayloadError;

    #[inline]
    fn poll(&mut self) -> Poll<Option<Self::Item>, PayloadError> {
        self.0.poll()
    }
}

/// Get request's payload stream
///
/// ## Example
///
/// ```rust
/// use futures::{Future, Stream};
/// use actix_web::{web, error, App, Error, HttpResponse};
///
/// /// extract binary data from request
/// fn index(body: web::Payload) -> impl Future<Item = HttpResponse, Error = Error>
/// {
///     body.map_err(Error::from)
///         .fold(web::BytesMut::new(), move |mut body, chunk| {
///             body.extend_from_slice(&chunk);
///             Ok::<_, Error>(body)
///          })
///          .and_then(|body| {
///              format!("Body {:?}!", body);
///              Ok(HttpResponse::Ok().finish())
///          })
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").route(
///             web::get().to_async(index))
///     );
/// }
/// ```
impl FromRequest for Payload {
    type Config = PayloadConfig;
    type Error = Error;
    type Future = Result<Payload, Error>;

    #[inline]
    fn from_request(_: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        Ok(Payload(payload.take()))
    }
}

/// Request binary data from a request's payload.
///
/// Loads request's payload and construct Bytes instance.
///
/// [**PayloadConfig**](struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// use bytes::Bytes;
/// use actix_web::{web, App};
///
/// /// extract binary data from request
/// fn index(body: Bytes) -> String {
///     format!("Body {:?}!", body)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").route(
///             web::get().to(index))
///     );
/// }
/// ```
impl FromRequest for Bytes {
    type Config = PayloadConfig;
    type Error = Error;
    type Future =
        Either<Box<dyn Future<Item = Bytes, Error = Error>>, FutureResult<Bytes, Error>>;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let tmp;
        let cfg = if let Some(cfg) = req.app_data::<PayloadConfig>() {
            cfg
        } else {
            tmp = PayloadConfig::default();
            &tmp
        };

        if let Err(e) = cfg.check_mimetype(req) {
            return Either::B(err(e));
        }

        let limit = cfg.limit;
        Either::A(Box::new(
            HttpMessageBody::new(req, payload).limit(limit).from_err(),
        ))
    }
}

/// Extract text information from a request's body.
///
/// Text extractor automatically decode body according to the request's charset.
///
/// [**PayloadConfig**](struct.PayloadConfig.html) allows to configure
/// extraction process.
///
/// ## Example
///
/// ```rust
/// use actix_web::{web, App, FromRequest};
///
/// /// extract text data from request
/// fn index(text: String) -> String {
///     format!("Body {}!", text)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html")
///             .data(String::configure(|cfg| {  // <- limit size of the payload
///                 cfg.limit(4096)
///             }))
///             .route(web::get().to(index))  // <- register handler with extractor params
///     );
/// }
/// ```
impl FromRequest for String {
    type Config = PayloadConfig;
    type Error = Error;
    type Future = Either<
        Box<dyn Future<Item = String, Error = Error>>,
        FutureResult<String, Error>,
    >;

    #[inline]
    fn from_request(req: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let tmp;
        let cfg = if let Some(cfg) = req.app_data::<PayloadConfig>() {
            cfg
        } else {
            tmp = PayloadConfig::default();
            &tmp
        };

        // check content-type
        if let Err(e) = cfg.check_mimetype(req) {
            return Either::B(err(e));
        }

        // check charset
        let encoding = match req.encoding() {
            Ok(enc) => enc,
            Err(e) => return Either::B(err(e.into())),
        };
        let limit = cfg.limit;

        Either::A(Box::new(
            HttpMessageBody::new(req, payload)
                .limit(limit)
                .from_err()
                .and_then(move |body| {
                    if encoding == UTF_8 {
                        Ok(str::from_utf8(body.as_ref())
                            .map_err(|_| ErrorBadRequest("Can not decode body"))?
                            .to_owned())
                    } else {
                        Ok(encoding
                            .decode_without_bom_handling_and_without_replacement(&body)
                            .map(|s| s.into_owned())
                            .ok_or_else(|| ErrorBadRequest("Can not decode body"))?)
                    }
                }),
        ))
    }
}
/// Payload configuration for request's payload.
#[derive(Clone)]
pub struct PayloadConfig {
    limit: usize,
    mimetype: Option<Mime>,
}

impl PayloadConfig {
    /// Create `PayloadConfig` instance and set max size of payload.
    pub fn new(limit: usize) -> Self {
        let mut cfg = Self::default();
        cfg.limit = limit;
        cfg
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set required mime-type of the request. By default mime type is not
    /// enforced.
    pub fn mimetype(mut self, mt: Mime) -> Self {
        self.mimetype = Some(mt);
        self
    }

    fn check_mimetype(&self, req: &HttpRequest) -> Result<(), Error> {
        // check content-type
        if let Some(ref mt) = self.mimetype {
            match req.mime_type() {
                Ok(Some(ref req_mt)) => {
                    if mt != req_mt {
                        return Err(ErrorBadRequest("Unexpected Content-Type"));
                    }
                }
                Ok(None) => {
                    return Err(ErrorBadRequest("Content-Type is expected"));
                }
                Err(err) => {
                    return Err(err.into());
                }
            }
        }
        Ok(())
    }
}

impl Default for PayloadConfig {
    fn default() -> Self {
        PayloadConfig {
            limit: 262_144,
            mimetype: None,
        }
    }
}

/// Future that resolves to a complete http message body.
///
/// Load http message body.
///
/// By default only 256Kb payload reads to a memory, then
/// `PayloadError::Overflow` get returned. Use `MessageBody::limit()`
/// method to change upper limit.
pub struct HttpMessageBody {
    limit: usize,
    length: Option<usize>,
    stream: Option<dev::Decompress<dev::Payload>>,
    err: Option<PayloadError>,
    fut: Option<Box<dyn Future<Item = Bytes, Error = PayloadError>>>,
}

impl HttpMessageBody {
    /// Create `MessageBody` for request.
    pub fn new(req: &HttpRequest, payload: &mut dev::Payload) -> HttpMessageBody {
        let mut len = None;
        if let Some(l) = req.headers().get(&header::CONTENT_LENGTH) {
            if let Ok(s) = l.to_str() {
                if let Ok(l) = s.parse::<usize>() {
                    len = Some(l)
                } else {
                    return Self::err(PayloadError::UnknownLength);
                }
            } else {
                return Self::err(PayloadError::UnknownLength);
            }
        }

        HttpMessageBody {
            stream: Some(dev::Decompress::from_headers(payload.take(), req.headers())),
            limit: 262_144,
            length: len,
            fut: None,
            err: None,
        }
    }

    /// Change max size of payload. By default max size is 256Kb
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    fn err(e: PayloadError) -> Self {
        HttpMessageBody {
            stream: None,
            limit: 262_144,
            fut: None,
            err: Some(e),
            length: None,
        }
    }
}

impl Future for HttpMessageBody {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut {
            return fut.poll();
        }

        if let Some(err) = self.err.take() {
            return Err(err);
        }

        if let Some(len) = self.length.take() {
            if len > self.limit {
                return Err(PayloadError::Overflow);
            }
        }

        // future
        let limit = self.limit;
        self.fut = Some(Box::new(
            self.stream
                .take()
                .unwrap()
                .from_err()
                .fold(BytesMut::with_capacity(8192), move |mut body, chunk| {
                    if (body.len() + chunk.len()) > limit {
                        Err(PayloadError::Overflow)
                    } else {
                        body.extend_from_slice(&chunk);
                        Ok(body)
                    }
                })
                .map(|body| body.freeze()),
        ));
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::http::header;
    use crate::test::{block_on, TestRequest};

    #[test]
    fn test_payload_config() {
        let req = TestRequest::default().to_http_request();
        let cfg = PayloadConfig::default().mimetype(mime::APPLICATION_JSON);
        assert!(cfg.check_mimetype(&req).is_err());

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .to_http_request();
        assert!(cfg.check_mimetype(&req).is_err());

        let req = TestRequest::with_header(header::CONTENT_TYPE, "application/json")
            .to_http_request();
        assert!(cfg.check_mimetype(&req).is_ok());
    }

    #[test]
    fn test_bytes() {
        let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_http_parts();

        let s = block_on(Bytes::from_request(&req, &mut pl)).unwrap();
        assert_eq!(s, Bytes::from_static(b"hello=world"));
    }

    #[test]
    fn test_string() {
        let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_http_parts();

        let s = block_on(String::from_request(&req, &mut pl)).unwrap();
        assert_eq!(s, "hello=world");
    }

    #[test]
    fn test_message_body() {
        let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "xxxx")
            .to_srv_request()
            .into_parts();
        let res = block_on(HttpMessageBody::new(&req, &mut pl));
        match res.err().unwrap() {
            PayloadError::UnknownLength => (),
            _ => unreachable!("error"),
        }

        let (req, mut pl) = TestRequest::with_header(header::CONTENT_LENGTH, "1000000")
            .to_srv_request()
            .into_parts();
        let res = block_on(HttpMessageBody::new(&req, &mut pl));
        match res.err().unwrap() {
            PayloadError::Overflow => (),
            _ => unreachable!("error"),
        }

        let (req, mut pl) = TestRequest::default()
            .set_payload(Bytes::from_static(b"test"))
            .to_http_parts();
        let res = block_on(HttpMessageBody::new(&req, &mut pl));
        assert_eq!(res.ok().unwrap(), Bytes::from_static(b"test"));

        let (req, mut pl) = TestRequest::default()
            .set_payload(Bytes::from_static(b"11111111111111"))
            .to_http_parts();
        let res = block_on(HttpMessageBody::new(&req, &mut pl).limit(5));
        match res.err().unwrap() {
            PayloadError::Overflow => (),
            _ => unreachable!("error"),
        }
    }
}

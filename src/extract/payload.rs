//! Payload/Bytes/String extractors
use std::str;

use actix_http::dev::MessageBody;
use actix_http::error::{Error, ErrorBadRequest, PayloadError};
use actix_http::HttpMessage;
use bytes::Bytes;
use encoding::all::UTF_8;
use encoding::types::{DecoderTrap, Encoding};
use futures::future::{err, Either, FutureResult};
use futures::{Future, Poll, Stream};
use mime::Mime;

use crate::extract::FromRequest;
use crate::service::ServiceFromRequest;

/// Payload extractor returns request 's payload stream.
///
/// ## Example
///
/// ```rust
/// use futures::{Future, Stream};
/// use actix_web::{web, error, App, Error, HttpResponse};
///
/// /// extract binary data from request
/// fn index<P>(body: web::Payload<P>) -> impl Future<Item = HttpResponse, Error = Error>
/// where
///     P: Stream<Item = web::Bytes, Error = error::PayloadError>
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
pub struct Payload<T>(crate::dev::Payload<T>);

impl<T> Stream for Payload<T>
where
    T: Stream<Item = Bytes, Error = PayloadError>,
{
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
/// fn index<P>(body: web::Payload<P>) -> impl Future<Item = HttpResponse, Error = Error>
/// where
///     P: Stream<Item = web::Bytes, Error = error::PayloadError>
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
impl<P> FromRequest<P> for Payload<P>
where
    P: Stream<Item = Bytes, Error = PayloadError>,
{
    type Error = Error;
    type Future = Result<Payload<P>, Error>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        Ok(Payload(req.take_payload()))
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
impl<P> FromRequest<P> for Bytes
where
    P: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Error = Error;
    type Future =
        Either<Box<Future<Item = Bytes, Error = Error>>, FutureResult<Bytes, Error>>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let mut tmp;
        let cfg = if let Some(cfg) = req.load_config::<PayloadConfig>() {
            cfg
        } else {
            tmp = PayloadConfig::default();
            &tmp
        };

        if let Err(e) = cfg.check_mimetype(req) {
            return Either::B(err(e));
        }

        let limit = cfg.limit;
        Either::A(Box::new(MessageBody::new(req).limit(limit).from_err()))
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
/// use actix_web::{web, App};
///
/// /// extract text data from request
/// fn index(text: String) -> String {
///     format!("Body {}!", text)
/// }
///
/// fn main() {
///     let app = App::new().service(
///         web::resource("/index.html").route(
///             web::get()
///                .config(web::PayloadConfig::new(4096)) // <- limit size of the payload
///                .to(index))  // <- register handler with extractor params
///     );
/// }
/// ```
impl<P> FromRequest<P> for String
where
    P: Stream<Item = Bytes, Error = PayloadError> + 'static,
{
    type Error = Error;
    type Future =
        Either<Box<Future<Item = String, Error = Error>>, FutureResult<String, Error>>;

    #[inline]
    fn from_request(req: &mut ServiceFromRequest<P>) -> Self::Future {
        let mut tmp;
        let cfg = if let Some(cfg) = req.load_config::<PayloadConfig>() {
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
            MessageBody::new(req)
                .limit(limit)
                .from_err()
                .and_then(move |body| {
                    let enc: *const Encoding = encoding as *const Encoding;
                    if enc == UTF_8 {
                        Ok(str::from_utf8(body.as_ref())
                            .map_err(|_| ErrorBadRequest("Can not decode body"))?
                            .to_owned())
                    } else {
                        Ok(encoding
                            .decode(&body, DecoderTrap::Strict)
                            .map_err(|_| ErrorBadRequest("Can not decode body"))?)
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
        Self::default().limit(limit)
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

    fn check_mimetype<P>(&self, req: &ServiceFromRequest<P>) -> Result<(), Error> {
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

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::http::header;
    use crate::test::{block_on, TestRequest};

    #[test]
    fn test_payload_config() {
        let req = TestRequest::default().to_from();
        let cfg = PayloadConfig::default().mimetype(mime::APPLICATION_JSON);
        assert!(cfg.check_mimetype(&req).is_err());

        let req = TestRequest::with_header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .to_from();
        assert!(cfg.check_mimetype(&req).is_err());

        let req =
            TestRequest::with_header(header::CONTENT_TYPE, "application/json").to_from();
        assert!(cfg.check_mimetype(&req).is_ok());
    }

    #[test]
    fn test_bytes() {
        let mut req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_from();

        let s = block_on(Bytes::from_request(&mut req)).unwrap();
        assert_eq!(s, Bytes::from_static(b"hello=world"));
    }

    #[test]
    fn test_string() {
        let mut req = TestRequest::with_header(header::CONTENT_LENGTH, "11")
            .set_payload(Bytes::from_static(b"hello=world"))
            .to_from();

        let s = block_on(String::from_request(&mut req)).unwrap();
        assert_eq!(s, "hello=world");
    }
}

//! `Middleware` for compressing response body.
use std::cmp;
use std::marker::PhantomData;
use std::str::FromStr;

use actix_http::body::MessageBody;
use actix_http::encoding::Encoder;
use actix_http::http::header::{ContentEncoding, ACCEPT_ENCODING};
use actix_http::{Error, Response, ResponseBuilder};
use actix_service::{Service, Transform};
use futures::future::{ok, FutureResult};
use futures::{Async, Future, Poll};

use crate::service::{ServiceRequest, ServiceResponse};

struct Enc(ContentEncoding);

/// Helper trait that allows to set specific encoding for response.
pub trait BodyEncoding {
    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self;
}

impl BodyEncoding for ResponseBuilder {
    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(Enc(encoding));
        self
    }
}

impl<B> BodyEncoding for Response<B> {
    fn encoding(&mut self, encoding: ContentEncoding) -> &mut Self {
        self.extensions_mut().insert(Enc(encoding));
        self
    }
}

#[derive(Debug, Clone)]
/// `Middleware` for compressing response body.
///
/// Use `BodyEncoding` trait for overriding response compression.
/// To disable compression set encoding to `ContentEncoding::Identity` value.
///
/// ```rust
/// use actix_web::{web, middleware, App, HttpResponse};
///
/// fn main() {
///     let app = App::new()
///         .wrap(middleware::Compress::default())
///         .service(
///             web::resource("/test")
///                 .route(web::get().to(|| HttpResponse::Ok()))
///                 .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
///         );
/// }
/// ```
pub struct Compress(ContentEncoding);

impl Compress {
    /// Create new `Compress` middleware with default encoding.
    pub fn new(encoding: ContentEncoding) -> Self {
        Compress(encoding)
    }
}

impl Default for Compress {
    fn default() -> Self {
        Compress::new(ContentEncoding::Auto)
    }
}

impl<S, B> Transform<S> for Compress
where
    B: MessageBody,
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<Encoder<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = CompressMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(CompressMiddleware {
            service,
            encoding: self.0,
        })
    }
}

pub struct CompressMiddleware<S> {
    service: S,
    encoding: ContentEncoding,
}

impl<S, B> Service for CompressMiddleware<S>
where
    B: MessageBody,
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<Encoder<B>>;
    type Error = Error;
    type Future = CompressResponse<S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        // negotiate content-encoding
        let encoding = if let Some(val) = req.headers().get(&ACCEPT_ENCODING) {
            if let Ok(enc) = val.to_str() {
                AcceptEncoding::parse(enc, self.encoding)
            } else {
                ContentEncoding::Identity
            }
        } else {
            ContentEncoding::Identity
        };

        CompressResponse {
            encoding,
            fut: self.service.call(req),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct CompressResponse<S, B>
where
    S: Service,
    B: MessageBody,
{
    fut: S::Future,
    encoding: ContentEncoding,
    _t: PhantomData<(B)>,
}

impl<S, B> Future for CompressResponse<S, B>
where
    B: MessageBody,
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Item = ServiceResponse<Encoder<B>>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let resp = futures::try_ready!(self.fut.poll());

        let enc = if let Some(enc) = resp.response().extensions().get::<Enc>() {
            enc.0
        } else {
            self.encoding
        };

        Ok(Async::Ready(resp.map_body(move |head, body| {
            Encoder::response(enc, head, body)
        })))
    }
}

struct AcceptEncoding {
    encoding: ContentEncoding,
    quality: f64,
}

impl Eq for AcceptEncoding {}

impl Ord for AcceptEncoding {
    fn cmp(&self, other: &AcceptEncoding) -> cmp::Ordering {
        if self.quality > other.quality {
            cmp::Ordering::Less
        } else if self.quality < other.quality {
            cmp::Ordering::Greater
        } else {
            cmp::Ordering::Equal
        }
    }
}

impl PartialOrd for AcceptEncoding {
    fn partial_cmp(&self, other: &AcceptEncoding) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AcceptEncoding {
    fn eq(&self, other: &AcceptEncoding) -> bool {
        self.quality == other.quality
    }
}

impl AcceptEncoding {
    fn new(tag: &str) -> Option<AcceptEncoding> {
        let parts: Vec<&str> = tag.split(';').collect();
        let encoding = match parts.len() {
            0 => return None,
            _ => ContentEncoding::from(parts[0]),
        };
        let quality = match parts.len() {
            1 => encoding.quality(),
            _ => match f64::from_str(parts[1]) {
                Ok(q) => q,
                Err(_) => 0.0,
            },
        };
        Some(AcceptEncoding { encoding, quality })
    }

    /// Parse a raw Accept-Encoding header value into an ordered list.
    pub fn parse(raw: &str, encoding: ContentEncoding) -> ContentEncoding {
        let mut encodings: Vec<_> = raw
            .replace(' ', "")
            .split(',')
            .map(|l| AcceptEncoding::new(l))
            .collect();
        encodings.sort();

        for enc in encodings {
            if let Some(enc) = enc {
                if encoding == ContentEncoding::Auto {
                    return enc.encoding;
                } else if encoding == enc.encoding {
                    return encoding;
                }
            }
        }
        ContentEncoding::Identity
    }
}

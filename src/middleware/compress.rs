//! For middleware documentation, see [`Compress`].

use std::{
    cmp,
    convert::TryFrom,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{
    body::{BoxBody, EitherBody, MessageBody},
    encoding::Encoder,
    http::header::{ContentEncoding, ACCEPT_ENCODING},
    StatusCode,
};
use actix_service::{Service, Transform};
use actix_utils::future::{ok, Either, Ready};
use futures_core::ready;
use once_cell::sync::Lazy;
use pin_project_lite::pin_project;

use crate::{
    dev::BodyEncoding,
    service::{ServiceRequest, ServiceResponse},
    Error, HttpResponse,
};

/// Middleware for compressing response payloads.
///
/// Use `BodyEncoding` trait for overriding response compression. To disable compression set
/// encoding to `ContentEncoding::Identity`.
///
/// # Examples
/// ```
/// use actix_web::{web, middleware, App, HttpResponse};
///
/// let app = App::new()
///     .wrap(middleware::Compress::default())
///     .default_service(web::to(|| HttpResponse::NotFound()));
/// ```
#[derive(Debug, Clone)]
pub struct Compress(ContentEncoding);

impl Compress {
    /// Create new `Compress` middleware with the specified encoding.
    pub fn new(encoding: ContentEncoding) -> Self {
        Compress(encoding)
    }
}

impl Default for Compress {
    fn default() -> Self {
        Compress::new(ContentEncoding::Auto)
    }
}

impl<S, B> Transform<S, ServiceRequest> for Compress
where
    B: MessageBody,
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Response = ServiceResponse<EitherBody<Encoder<B>>>;
    type Error = Error;
    type Transform = CompressMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

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

static SUPPORTED_ALGORITHM_NAMES: Lazy<String> = Lazy::new(|| {
    #[allow(unused_mut)] // only unused when no compress features enabled
    let mut encoding: Vec<&str> = vec![];

    #[cfg(feature = "compress-brotli")]
    {
        encoding.push("br");
    }

    #[cfg(feature = "compress-gzip")]
    {
        encoding.push("gzip");
        encoding.push("deflate");
    }

    #[cfg(feature = "compress-zstd")]
    encoding.push("zstd");

    assert!(
        !encoding.is_empty(),
        "encoding can not be empty unless __compress feature has been explicitly enabled by itself"
    );

    encoding.join(", ")
});

impl<S, B> Service<ServiceRequest> for CompressMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    type Response = ServiceResponse<EitherBody<Encoder<B>>>;
    type Error = Error;
    type Future = Either<CompressResponse<S, B>, Ready<Result<Self::Response, Self::Error>>>;

    actix_service::forward_ready!(service);

    #[allow(clippy::borrow_interior_mutable_const)]
    fn call(&self, req: ServiceRequest) -> Self::Future {
        // negotiate content-encoding
        let encoding_result = req
            .headers()
            .get(&ACCEPT_ENCODING)
            .and_then(|val| val.to_str().ok())
            .map(|enc| AcceptEncoding::try_parse(enc, self.encoding));

        match encoding_result {
            // Missing header => fallback to identity
            None => Either::left(CompressResponse {
                encoding: ContentEncoding::Identity,
                fut: self.service.call(req),
                _phantom: PhantomData,
            }),

            // Valid encoding
            Some(Ok(encoding)) => Either::left(CompressResponse {
                encoding,
                fut: self.service.call(req),
                _phantom: PhantomData,
            }),

            // There is an HTTP header but we cannot match what client as asked for
            Some(Err(_)) => {
                let res = HttpResponse::with_body(
                    StatusCode::NOT_ACCEPTABLE,
                    SUPPORTED_ALGORITHM_NAMES.clone(),
                );

                Either::right(ok(req
                    .into_response(res)
                    .map_body(|_, body| EitherBody::right(BoxBody::new(body)))))
            }
        }
    }
}

pin_project! {
    pub struct CompressResponse<S, B>
    where
        S: Service<ServiceRequest>,
    {
        #[pin]
        fut: S::Future,
        encoding: ContentEncoding,
        _phantom: PhantomData<B>,
    }
}

impl<S, B> Future for CompressResponse<S, B>
where
    B: MessageBody,
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Output = Result<ServiceResponse<EitherBody<Encoder<B>>>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match ready!(this.fut.poll(cx)) {
            Ok(resp) => {
                let enc = if let Some(enc) = resp.response().get_encoding() {
                    enc
                } else {
                    *this.encoding
                };

                Poll::Ready(Ok(resp.map_body(move |head, body| {
                    EitherBody::left(Encoder::response(enc, head, body))
                })))
            }

            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

struct AcceptEncoding {
    encoding: ContentEncoding,
    // TODO: use Quality or QualityItem<ContentEncoding>
    quality: f64,
}

impl Eq for AcceptEncoding {}

impl Ord for AcceptEncoding {
    #[allow(clippy::comparison_chain)]
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
        self.encoding == other.encoding && self.quality == other.quality
    }
}

/// Parse q-factor from quality strings.
///
/// If parse fail, then fallback to default value which is 1.
/// More details available here: <https://developer.mozilla.org/en-US/docs/Glossary/Quality_values>
fn parse_quality(parts: &[&str]) -> f64 {
    for part in parts {
        if part.trim().starts_with("q=") {
            return part[2..].parse().unwrap_or(1.0);
        }
    }

    1.0
}

#[derive(Debug, PartialEq, Eq)]
enum AcceptEncodingError {
    /// This error occurs when client only support compressed response and server do not have any
    /// algorithm that match client accepted algorithms.
    CompressionAlgorithmMismatch,
}

impl AcceptEncoding {
    fn new(tag: &str) -> Option<AcceptEncoding> {
        let parts: Vec<&str> = tag.split(';').collect();
        let encoding = match parts.len() {
            0 => return None,
            _ => match ContentEncoding::try_from(parts[0]) {
                Err(_) => return None,
                Ok(x) => x,
            },
        };

        let quality = parse_quality(&parts[1..]);
        if quality <= 0.0 || quality > 1.0 {
            return None;
        }

        Some(AcceptEncoding { encoding, quality })
    }

    /// Parse a raw Accept-Encoding header value into an ordered list then return the best match
    /// based on middleware configuration.
    pub fn try_parse(
        raw: &str,
        encoding: ContentEncoding,
    ) -> Result<ContentEncoding, AcceptEncodingError> {
        let mut encodings = raw
            .replace(' ', "")
            .split(',')
            .filter_map(AcceptEncoding::new)
            .collect::<Vec<_>>();

        encodings.sort();

        for enc in encodings {
            if encoding == ContentEncoding::Auto || encoding == enc.encoding {
                return Ok(enc.encoding);
            }
        }

        // Special case if user cannot accept uncompressed data.
        // See: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Accept-Encoding
        // TODO: account for whitespace
        if raw.contains("*;q=0") || raw.contains("identity;q=0") {
            return Err(AcceptEncodingError::CompressionAlgorithmMismatch);
        }

        Ok(ContentEncoding::Identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! assert_parse_eq {
        ($raw:expr, $result:expr) => {
            assert_eq!(
                AcceptEncoding::try_parse($raw, ContentEncoding::Auto),
                Ok($result)
            );
        };
    }

    macro_rules! assert_parse_fail {
        ($raw:expr) => {
            assert!(AcceptEncoding::try_parse($raw, ContentEncoding::Auto).is_err());
        };
    }

    #[test]
    fn test_parse_encoding() {
        // Test simple case
        assert_parse_eq!("br", ContentEncoding::Br);
        assert_parse_eq!("gzip", ContentEncoding::Gzip);
        assert_parse_eq!("deflate", ContentEncoding::Deflate);
        assert_parse_eq!("zstd", ContentEncoding::Zstd);

        // Test space, trim, missing values
        assert_parse_eq!("br,,,,", ContentEncoding::Br);
        assert_parse_eq!("gzip  ,   br,   zstd", ContentEncoding::Gzip);

        // Test float number parsing
        assert_parse_eq!("br;q=1  ,", ContentEncoding::Br);
        assert_parse_eq!("br;q=1.0  ,   br", ContentEncoding::Br);

        // Test wildcard
        assert_parse_eq!("*", ContentEncoding::Identity);
        assert_parse_eq!("*;q=1.0", ContentEncoding::Identity);
    }

    #[test]
    fn test_parse_encoding_qfactor_ordering() {
        assert_parse_eq!("gzip, br, zstd", ContentEncoding::Gzip);
        assert_parse_eq!("zstd, br, gzip", ContentEncoding::Zstd);

        assert_parse_eq!("gzip;q=0.4, br;q=0.6", ContentEncoding::Br);
        assert_parse_eq!("gzip;q=0.8, br;q=0.4", ContentEncoding::Gzip);
    }

    #[test]
    fn test_parse_encoding_qfactor_invalid() {
        // Out of range
        assert_parse_eq!("gzip;q=-5.0", ContentEncoding::Identity);
        assert_parse_eq!("gzip;q=5.0", ContentEncoding::Identity);

        // Disabled
        assert_parse_eq!("gzip;q=0", ContentEncoding::Identity);
    }

    #[test]
    fn test_parse_compression_required() {
        // Check we fallback to identity if there is an unsupported compression algorithm
        assert_parse_eq!("compress", ContentEncoding::Identity);

        // User do not want any compression
        assert_parse_fail!("compress, identity;q=0");
        assert_parse_fail!("compress, identity;q=0.0");
        assert_parse_fail!("compress, *;q=0");
        assert_parse_fail!("compress, *;q=0.0");
    }
}

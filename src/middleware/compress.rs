//! For middleware documentation, see [`Compress`].

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::encoding::Encoder;
use actix_service::{Service, Transform};
use actix_utils::future::{ok, Either, Ready};
use futures_core::ready;
use once_cell::sync::Lazy;
use pin_project_lite::pin_project;

use crate::{
    body::{EitherBody, MessageBody},
    dev::BodyEncoding,
    http::{
        header::{self, AcceptEncoding, ContentEncoding, Encoding, HeaderValue},
        StatusCode,
    },
    service::{ServiceRequest, ServiceResponse},
    Error, HttpMessage, HttpResponse,
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
#[non_exhaustive]
pub struct Compress;

impl Compress {
    /// Create new `Compress` middleware with the specified encoding.
    // TODO: remove
    pub fn new(encoding: ContentEncoding) -> Self {
        // Compress(encoding)
        Compress
    }
}

impl Default for Compress {
    fn default() -> Self {
        // Compress::new(ContentEncoding::Auto)
        Compress
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
            // encoding: self.0,
        })
    }
}

pub struct CompressMiddleware<S> {
    service: S,
    // encoding: ContentEncoding,
}

static SUPPORTED_ENCODINGS_STRING: Lazy<String> = Lazy::new(|| {
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
    {
        encoding.push("zstd");
    }

    assert!(
        !encoding.is_empty(),
        "encoding can not be empty unless __compress feature has been explicitly enabled by itself"
    );

    encoding.join(", ")
});

static SUPPORTED_ENCODINGS: Lazy<Vec<Encoding>> = Lazy::new(|| {
    let mut encodings = vec![Encoding::Identity];

    #[cfg(feature = "compress-brotli")]
    {
        encodings.push(Encoding::Brotli);
    }

    #[cfg(feature = "compress-gzip")]
    {
        encodings.push(Encoding::Gzip);
        encodings.push(Encoding::Deflate);
    }

    #[cfg(feature = "compress-zstd")]
    {
        encodings.push(Encoding::Zstd);
    }

    assert!(
        !encodings.is_empty(),
        "encodings can not be empty unless __compress feature has been explicitly enabled by itself"
    );

    encodings
});

impl<S, B> Service<ServiceRequest> for CompressMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    type Response = ServiceResponse<EitherBody<Encoder<B>>>;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future = Either<CompressResponse<S, B>, Ready<Result<Self::Response, Self::Error>>>;

    actix_service::forward_ready!(service);

    #[allow(clippy::borrow_interior_mutable_const)]
    fn call(&self, req: ServiceRequest) -> Self::Future {
        // negotiate content-encoding
        let accept_encoding = req.get_header::<AcceptEncoding>();

        let accept_encoding = match accept_encoding {
            // missing header; fallback to identity
            None => {
                return Either::left(CompressResponse {
                    encoding: Encoding::Identity,
                    fut: self.service.call(req),
                    _phantom: PhantomData,
                })
            }

            // valid accept-encoding header
            Some(accept_encoding) => accept_encoding,
        };

        match accept_encoding.negotiate(SUPPORTED_ENCODINGS.iter()) {
            None => {
                let mut res = HttpResponse::with_body(
                    StatusCode::NOT_ACCEPTABLE,
                    SUPPORTED_ENCODINGS_STRING.as_str(),
                );

                res.headers_mut()
                    .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));

                Either::right(ok(req
                    .into_response(res)
                    .map_into_boxed_body()
                    .map_into_right_body()))
            }

            Some(encoding) => Either::left(CompressResponse {
                fut: self.service.call(req),
                encoding,
                _phantom: PhantomData,
            }),
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
        encoding: Encoding,
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
                    match this.encoding {
                        Encoding::Brotli => ContentEncoding::Brotli,
                        Encoding::Gzip => ContentEncoding::Gzip,
                        Encoding::Deflate => ContentEncoding::Deflate,
                        Encoding::Identity => ContentEncoding::Identity,
                        Encoding::Zstd => ContentEncoding::Zstd,
                        enc => unimplemented!("encoding {} should not be here", enc),
                    }
                };

                Poll::Ready(Ok(resp.map_body(move |head, body| {
                    EitherBody::left(Encoder::response(enc, head, body))
                })))
            }

            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

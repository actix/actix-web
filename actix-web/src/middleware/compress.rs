//! For middleware documentation, see [`Compress`].

use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::encoding::Encoder;
use actix_service::{Service, Transform};
use actix_utils::future::{ok, Either, Ready};
use futures_core::ready;
use mime::Mime;
use once_cell::sync::Lazy;
use pin_project_lite::pin_project;

use crate::{
    body::{EitherBody, MessageBody},
    http::{
        header::{self, AcceptEncoding, ContentEncoding, Encoding, HeaderValue},
        StatusCode,
    },
    service::{ServiceRequest, ServiceResponse},
    Error, HttpMessage, HttpResponse,
};

/// Middleware for compressing response payloads.
///
/// # Encoding Negotiation
/// `Compress` will read the `Accept-Encoding` header to negotiate which compression codec to use.
/// Payloads are not compressed if the header is not sent. The `compress-*` [feature flags] are also
/// considered in this selection process.
///
/// # Pre-compressed Payload
/// If you are serving some data is already using a compressed representation (e.g., a gzip
/// compressed HTML file from disk) you can signal this to `Compress` by setting an appropriate
/// `Content-Encoding` header. In addition to preventing double compressing the payload, this header
/// is required by the spec when using compressed representations and will inform the client that
/// the content should be uncompressed.
///
/// However, it is not advised to unconditionally serve encoded representations of content because
/// the client may not support it. The [`AcceptEncoding`] typed header has some utilities to help
/// perform manual encoding negotiation, if required. When negotiating content encoding, it is also
/// required by the spec to send a `Vary: Accept-Encoding` header.
///
/// A (naïve) example serving an pre-compressed Gzip file is included below.
///
/// # Examples
/// To enable automatic payload compression just include `Compress` as a top-level middleware:
/// ```
/// use actix_web::{middleware, web, App, HttpResponse};
///
/// let app = App::new()
///     .wrap(middleware::Compress::default())
///     .default_service(web::to(|| async { HttpResponse::Ok().body("hello world") }));
/// ```
///
/// Pre-compressed Gzip file being served from disk with correct headers added to bypass middleware:
/// ```no_run
/// use actix_web::{middleware, http::header, web, App, HttpResponse, Responder};
///
/// async fn index_handler() -> actix_web::Result<impl Responder> {
///     Ok(actix_files::NamedFile::open_async("./assets/index.html.gz").await?
///         .customize()
///         .insert_header(header::ContentEncoding::Gzip))
/// }
///
/// let app = App::new()
///     .wrap(middleware::Compress::default())
///     .default_service(web::to(index_handler));
/// ```
///
/// [feature flags]: ../index.html#crate-features
#[derive(Clone)]
#[non_exhaustive]
pub struct Compress {
    pub compress: fn(&HeaderValue) -> bool,
}

impl fmt::Debug for Compress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compress").finish()
    }
}
impl Default for Compress {
    fn default() -> Self {
        Compress {
            compress: |_| false,
        }
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
            compress: self.compress,
        })
    }
}

pub struct CompressMiddleware<S> {
    service: S,
    compress: fn(&HeaderValue) -> bool,
}

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
                    encoding: Encoding::identity(),
                    fut: self.service.call(req),
                    _phantom: PhantomData,
                    compress: self.compress,
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
                compress: self.compress,
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
    compress: fn(&HeaderValue) -> bool,
    }
}

impl<S, B> Future for CompressResponse<S, B>
where
    B: MessageBody,
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Output = Result<ServiceResponse<EitherBody<Encoder<B>>>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();

        match ready!(this.fut.poll(cx)) {
            Ok(resp) => {
                let enc = match this.encoding {
                    Encoding::Known(enc) => *enc,
                    Encoding::Unknown(enc) => {
                        unimplemented!("encoding {} should not be here", enc);
                    }
                };

                Poll::Ready(Ok(resp.map_body(move |head, body| {
                    let content_type = head.headers.get(header::CONTENT_TYPE);
                    let should_compress = content_type
                        .map(|value| (self.compress)(value))
                        .unwrap_or(true);
                    if should_compress {
                        EitherBody::left(Encoder::response(enc, head, body))
                    } else {
                        EitherBody::left(Encoder::response(
                            ContentEncoding::Identity,
                            head,
                            body,
                        ))
                    }
                })))
            }

            Err(err) => Poll::Ready(Err(err)),
        }
    }
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

static SUPPORTED_ENCODINGS: &[Encoding] = &[
    Encoding::identity(),
    #[cfg(feature = "compress-brotli")]
    {
        Encoding::brotli()
    },
    #[cfg(feature = "compress-gzip")]
    {
        Encoding::gzip()
    },
    #[cfg(feature = "compress-gzip")]
    {
        Encoding::deflate()
    },
    #[cfg(feature = "compress-zstd")]
    {
        Encoding::zstd()
    },
];

// move cfg(feature) to prevents_double_compressing if more tests are added
#[cfg(feature = "compress-gzip")]
#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::http::header::ContentType;
    use crate::{middleware::DefaultHeaders, test, web, App};

    pub fn gzip_decode(bytes: impl AsRef<[u8]>) -> Vec<u8> {
        use std::io::Read as _;
        let mut decoder = flate2::read::GzDecoder::new(bytes.as_ref());
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).unwrap();
        buf
    }

    #[actix_rt::test]
    async fn prevents_double_compressing() {
        const D: &str = "hello world ";
        const DATA: &str = const_str::repeat!(D, 100);

        let app = test::init_service({
            App::new()
                .wrap(Compress::default())
                .route(
                    "/single",
                    web::get().to(move || HttpResponse::Ok().body(DATA)),
                )
                .service(
                    web::resource("/double")
                        .wrap(Compress::default())
                        .wrap(DefaultHeaders::new().add(("x-double", "true")))
                        .route(web::get().to(move || HttpResponse::Ok().body(DATA))),
                )
        })
        .await;

        let req = test::TestRequest::default()
            .uri("/single")
            .insert_header((header::ACCEPT_ENCODING, "gzip"))
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get("x-double"), None);
        assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "gzip");
        let bytes = test::read_body(res).await;
        assert_eq!(gzip_decode(bytes), DATA.as_bytes());

        let req = test::TestRequest::default()
            .uri("/double")
            .insert_header((header::ACCEPT_ENCODING, "gzip"))
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get("x-double").unwrap(), "true");
        assert_eq!(res.headers().get(header::CONTENT_ENCODING).unwrap(), "gzip");
        let bytes = test::read_body(res).await;
        assert_eq!(gzip_decode(bytes), DATA.as_bytes());
    }

    #[actix_rt::test]
    async fn retains_previously_set_vary_header() {
        let app = test::init_service({
            App::new()
                .wrap(Compress::default())
                .default_service(web::to(move || {
                    HttpResponse::Ok()
                        .insert_header((header::VARY, "x-test"))
                        .finish()
                }))
        })
        .await;

        let req = test::TestRequest::default()
            .insert_header((header::ACCEPT_ENCODING, "gzip"))
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        #[allow(clippy::mutable_key_type)]
        let vary_headers = res.headers().get_all(header::VARY).collect::<HashSet<_>>();
        assert!(vary_headers.contains(&HeaderValue::from_static("x-test")));
        assert!(vary_headers.contains(&HeaderValue::from_static("accept-encoding")));
    }

    #[actix_rt::test]
    async fn prevents_compression_jpeg() {
        const D: &str = "test image";
        const DATA: &str = const_str::repeat!(D, 100);
        let app = test::init_service({
            App::new().wrap(Compress::default()).route(
                "/image",
                web::get().to(move || {
                    let builder = HttpResponse::Ok()
                        .insert_header(ContentType::jpeg())
                        .body(DATA);
                    builder
                }),
            )
        })
        .await;
        let req = test::TestRequest::default()
            .uri("/image")
            .insert_header((header::ACCEPT_ENCODING, "gzip"))
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
        let bytes = test::read_body(res).await;
        assert_eq!(bytes, DATA.as_bytes());
    }
}

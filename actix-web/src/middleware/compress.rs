//! For middleware documentation, see [`Compress`].

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_http::{encoding::Encoder, header::ContentEncoding};
use actix_service::{Service, Transform};
use actix_utils::future::{ok, Either, Ready};
use futures_core::ready;
use once_cell::sync::Lazy;
use pin_project_lite::pin_project;

use crate::{
    body::{EitherBody, MessageBody},
    http::{
        header::{self, AcceptEncoding, Encoding, HeaderValue},
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
/// You can also set compression level for supported algorithms
/// ```
/// use actix_web::{middleware, web, App, HttpResponse};
///
/// let app = App::new()
///     .wrap(
///         middleware::Compress::new()
///             .set_gzip_level(3)
///             .set_deflate_level(1)
///             .set_brotli_level(7)
///             .set_zstd_level(10),
///     )
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
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Compress {
    inner: Rc<Inner>,
}

impl Compress {
    /// Constructs new compress middleware instance with default settings.
    pub fn new() -> Self {
        Default::default()
    }
}

#[derive(Debug, Clone, Default)]
struct Inner {
    deflate: Option<u32>,
    gzip: Option<u32>,
    brotli: Option<u32>,
    zstd: Option<u32>,
}

impl Inner {
    pub fn level(&self, encoding: &ContentEncoding) -> Option<u32> {
        match encoding {
            ContentEncoding::Deflate => self.deflate,
            ContentEncoding::Gzip => self.gzip,
            ContentEncoding::Brotli => self.brotli,
            ContentEncoding::Zstd => self.zstd,
            _ => None,
        }
    }
}

impl Compress {
    /// Set deflate compression level.
    ///
    /// The integer here is on a scale of 0-9.
    /// When going out of range, level 1 will be used.
    pub fn set_deflate_level(mut self, value: u32) -> Self {
        Rc::get_mut(&mut self.inner).unwrap().deflate = Some(value);
        self
    }
    /// Set gzip compression level.
    ///
    /// The integer here is on a scale of 0-9.
    /// When going out of range, level 1 will be used.
    pub fn set_gzip_level(mut self, value: u32) -> Self {
        Rc::get_mut(&mut self.inner).unwrap().gzip = Some(value);
        self
    }
    /// Set gzip compression level.
    ///
    /// The integer here is on a scale of 0-11.
    /// When going out of range, level 3 will be used.
    pub fn set_brotli_level(mut self, value: u32) -> Self {
        Rc::get_mut(&mut self.inner).unwrap().brotli = Some(value);
        self
    }
    /// Set gzip compression level.
    ///
    /// The integer here is on a scale of 0-22.
    /// When going out of range, level 3 will be used.
    pub fn set_zstd_level(mut self, value: u32) -> Self {
        Rc::get_mut(&mut self.inner).unwrap().zstd = Some(value);
        self
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
            inner: Rc::clone(&self.inner),
        })
    }
}

pub struct CompressMiddleware<S> {
    service: S,
    inner: Rc<Inner>,
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
        let inner = self.inner.clone();

        let accept_encoding = match accept_encoding {
            // missing header; fallback to identity
            None => {
                return Either::left(CompressResponse {
                    encoding: Encoding::identity(),
                    fut: self.service.call(req),
                    inner,
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
                inner,
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
        inner: Rc<Inner>,
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
                let enc = match this.encoding {
                    Encoding::Known(enc) => *enc,
                    Encoding::Unknown(enc) => {
                        unimplemented!("encoding {} should not be here", enc);
                    }
                };
                let level = this.inner.level(&enc);

                Poll::Ready(Ok(resp.map_body(move |head, body| {
                    EitherBody::left(Encoder::response_with_level(enc, head, body, level))
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
    async fn custom_compress_level() {
        const D: &str = "hello world ";
        const DATA: &str = const_str::repeat!(D, 100);

        let app = test::init_service({
            App::new().wrap(Compress::new().set_gzip_level(9)).route(
                "/compress",
                web::get().to(move || HttpResponse::Ok().body(DATA)),
            )
        })
        .await;

        let req = test::TestRequest::default()
            .uri("/compress")
            .insert_header((header::ACCEPT_ENCODING, "gzip"))
            .to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = test::read_body(res).await;
        assert_eq!(gzip_decode(bytes), DATA.as_bytes());
    }
}

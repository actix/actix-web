#![allow(clippy::borrow_interior_mutable_const, clippy::type_complexity)]
//! Cross-origin resource sharing (CORS) for Actix applications
//!
//! CORS middleware could be used with application and with resource.
//! Cors middleware could be used as parameter for `App::wrap()`,
//! `Resource::wrap()` or `Scope::wrap()` methods.
//!
//! # Example
//!
//! ```rust
//! use actix_cors::Cors;
//! use actix_web::{http, web, App, HttpRequest, HttpResponse, HttpServer};
//!
//! fn index(req: HttpRequest) -> &'static str {
//!     "Hello world"
//! }
//!
//! fn main() -> std::io::Result<()> {
//!     HttpServer::new(|| App::new()
//!         .wrap(
//!             Cors::new() // <- Construct CORS middleware builder
//!               .allowed_origin("https://www.rust-lang.org/")
//!               .allowed_methods(vec!["GET", "POST"])
//!               .allowed_headers(vec![http::header::AUTHORIZATION, http::header::ACCEPT])
//!               .allowed_header(http::header::CONTENT_TYPE)
//!               .max_age(3600))
//!         .service(
//!             web::resource("/index.html")
//!               .route(web::get().to(index))
//!               .route(web::head().to(|| HttpResponse::MethodNotAllowed()))
//!         ))
//!         .bind("127.0.0.1:8080")?;
//!
//!     Ok(())
//! }
//! ```
//! In this example custom *CORS* middleware get registered for "/index.html"
//! endpoint.
//!
//! Cors middleware automatically handle *OPTIONS* preflight request.
use std::collections::HashSet;
use std::iter::FromIterator;
use std::rc::Rc;

use actix_service::{IntoTransform, Service, Transform};
use actix_web::dev::{RequestHead, ServiceRequest, ServiceResponse};
use actix_web::error::{Error, ResponseError, Result};
use actix_web::http::header::{self, HeaderName, HeaderValue};
use actix_web::http::{self, HttpTryFrom, Method, StatusCode, Uri};
use actix_web::HttpResponse;
use derive_more::Display;
use futures::future::{ok, Either, Future, FutureResult};
use futures::Poll;

/// A set of errors that can occur during processing CORS
#[derive(Debug, Display)]
pub enum CorsError {
    /// The HTTP request header `Origin` is required but was not provided
    #[display(
        fmt = "The HTTP request header `Origin` is required but was not provided"
    )]
    MissingOrigin,
    /// The HTTP request header `Origin` could not be parsed correctly.
    #[display(fmt = "The HTTP request header `Origin` could not be parsed correctly.")]
    BadOrigin,
    /// The request header `Access-Control-Request-Method` is required but is
    /// missing
    #[display(
        fmt = "The request header `Access-Control-Request-Method` is required but is missing"
    )]
    MissingRequestMethod,
    /// The request header `Access-Control-Request-Method` has an invalid value
    #[display(
        fmt = "The request header `Access-Control-Request-Method` has an invalid value"
    )]
    BadRequestMethod,
    /// The request header `Access-Control-Request-Headers`  has an invalid
    /// value
    #[display(
        fmt = "The request header `Access-Control-Request-Headers`  has an invalid value"
    )]
    BadRequestHeaders,
    /// Origin is not allowed to make this request
    #[display(fmt = "Origin is not allowed to make this request")]
    OriginNotAllowed,
    /// Requested method is not allowed
    #[display(fmt = "Requested method is not allowed")]
    MethodNotAllowed,
    /// One or more headers requested are not allowed
    #[display(fmt = "One or more headers requested are not allowed")]
    HeadersNotAllowed,
}

impl ResponseError for CorsError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::with_body(StatusCode::BAD_REQUEST, format!("{}", self).into())
    }
}

/// An enum signifying that some of type T is allowed, or `All` (everything is
/// allowed).
///
/// `Default` is implemented for this enum and is `All`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AllOrSome<T> {
    /// Everything is allowed. Usually equivalent to the "*" value.
    All,
    /// Only some of `T` is allowed
    Some(T),
}

impl<T> Default for AllOrSome<T> {
    fn default() -> Self {
        AllOrSome::All
    }
}

impl<T> AllOrSome<T> {
    /// Returns whether this is an `All` variant
    pub fn is_all(&self) -> bool {
        match *self {
            AllOrSome::All => true,
            AllOrSome::Some(_) => false,
        }
    }

    /// Returns whether this is a `Some` variant
    pub fn is_some(&self) -> bool {
        !self.is_all()
    }

    /// Returns &T
    pub fn as_ref(&self) -> Option<&T> {
        match *self {
            AllOrSome::All => None,
            AllOrSome::Some(ref t) => Some(t),
        }
    }
}

/// Structure that follows the builder pattern for building `Cors` middleware
/// structs.
///
/// To construct a cors:
///
///   1. Call [`Cors::build`](struct.Cors.html#method.build) to start building.
///   2. Use any of the builder methods to set fields in the backend.
/// 3. Call [finish](struct.Cors.html#method.finish) to retrieve the
/// constructed backend.
///
/// # Example
///
/// ```rust
/// use actix_cors::Cors;
/// use actix_web::http::header;
///
/// # fn main() {
/// let cors = Cors::new()
///     .allowed_origin("https://www.rust-lang.org/")
///     .allowed_methods(vec!["GET", "POST"])
///     .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
///     .allowed_header(header::CONTENT_TYPE)
///     .max_age(3600);
/// # }
/// ```
#[derive(Default)]
pub struct Cors {
    cors: Option<Inner>,
    methods: bool,
    error: Option<http::Error>,
    expose_hdrs: HashSet<HeaderName>,
}

impl Cors {
    /// Build a new CORS middleware instance
    pub fn new() -> Cors {
        Cors {
            cors: Some(Inner {
                origins: AllOrSome::All,
                origins_str: None,
                methods: HashSet::new(),
                headers: AllOrSome::All,
                expose_hdrs: None,
                max_age: None,
                preflight: true,
                send_wildcard: false,
                supports_credentials: false,
                vary_header: true,
            }),
            methods: false,
            error: None,
            expose_hdrs: HashSet::new(),
        }
    }

    /// Build a new CORS default middleware
    pub fn default() -> CorsFactory {
        let inner = Inner {
            origins: AllOrSome::default(),
            origins_str: None,
            methods: HashSet::from_iter(
                vec![
                    Method::GET,
                    Method::HEAD,
                    Method::POST,
                    Method::OPTIONS,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                ]
                .into_iter(),
            ),
            headers: AllOrSome::All,
            expose_hdrs: None,
            max_age: None,
            preflight: true,
            send_wildcard: false,
            supports_credentials: false,
            vary_header: true,
        };
        CorsFactory {
            inner: Rc::new(inner),
        }
    }

    /// Add an origin that are allowed to make requests.
    /// Will be verified against the `Origin` request header.
    ///
    /// When `All` is set, and `send_wildcard` is set, "*" will be sent in
    /// the `Access-Control-Allow-Origin` response header. Otherwise, the
    /// client's `Origin` request header will be echoed back in the
    /// `Access-Control-Allow-Origin` response header.
    ///
    /// When `Some` is set, the client's `Origin` request header will be
    /// checked in a case-sensitive manner.
    ///
    /// This is the `list of origins` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// Defaults to `All`.
    ///
    /// Builder panics if supplied origin is not valid uri.
    pub fn allowed_origin(mut self, origin: &str) -> Cors {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            match Uri::try_from(origin) {
                Ok(_) => {
                    if cors.origins.is_all() {
                        cors.origins = AllOrSome::Some(HashSet::new());
                    }
                    if let AllOrSome::Some(ref mut origins) = cors.origins {
                        origins.insert(origin.to_owned());
                    }
                }
                Err(e) => {
                    self.error = Some(e.into());
                }
            }
        }
        self
    }

    /// Set a list of methods which the allowed origins are allowed to access
    /// for requests.
    ///
    /// This is the `list of methods` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// Defaults to `[GET, HEAD, POST, OPTIONS, PUT, PATCH, DELETE]`
    pub fn allowed_methods<U, M>(mut self, methods: U) -> Cors
    where
        U: IntoIterator<Item = M>,
        Method: HttpTryFrom<M>,
    {
        self.methods = true;
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            for m in methods {
                match Method::try_from(m) {
                    Ok(method) => {
                        cors.methods.insert(method);
                    }
                    Err(e) => {
                        self.error = Some(e.into());
                        break;
                    }
                }
            }
        }
        self
    }

    /// Set an allowed header
    pub fn allowed_header<H>(mut self, header: H) -> Cors
    where
        HeaderName: HttpTryFrom<H>,
    {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            match HeaderName::try_from(header) {
                Ok(method) => {
                    if cors.headers.is_all() {
                        cors.headers = AllOrSome::Some(HashSet::new());
                    }
                    if let AllOrSome::Some(ref mut headers) = cors.headers {
                        headers.insert(method);
                    }
                }
                Err(e) => self.error = Some(e.into()),
            }
        }
        self
    }

    /// Set a list of header field names which can be used when
    /// this resource is accessed by allowed origins.
    ///
    /// If `All` is set, whatever is requested by the client in
    /// `Access-Control-Request-Headers` will be echoed back in the
    /// `Access-Control-Allow-Headers` header.
    ///
    /// This is the `list of headers` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// Defaults to `All`.
    pub fn allowed_headers<U, H>(mut self, headers: U) -> Cors
    where
        U: IntoIterator<Item = H>,
        HeaderName: HttpTryFrom<H>,
    {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            for h in headers {
                match HeaderName::try_from(h) {
                    Ok(method) => {
                        if cors.headers.is_all() {
                            cors.headers = AllOrSome::Some(HashSet::new());
                        }
                        if let AllOrSome::Some(ref mut headers) = cors.headers {
                            headers.insert(method);
                        }
                    }
                    Err(e) => {
                        self.error = Some(e.into());
                        break;
                    }
                }
            }
        }
        self
    }

    /// Set a list of headers which are safe to expose to the API of a CORS API
    /// specification. This corresponds to the
    /// `Access-Control-Expose-Headers` response header.
    ///
    /// This is the `list of exposed headers` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// This defaults to an empty set.
    pub fn expose_headers<U, H>(mut self, headers: U) -> Cors
    where
        U: IntoIterator<Item = H>,
        HeaderName: HttpTryFrom<H>,
    {
        for h in headers {
            match HeaderName::try_from(h) {
                Ok(method) => {
                    self.expose_hdrs.insert(method);
                }
                Err(e) => {
                    self.error = Some(e.into());
                    break;
                }
            }
        }
        self
    }

    /// Set a maximum time for which this CORS request maybe cached.
    /// This value is set as the `Access-Control-Max-Age` header.
    ///
    /// This defaults to `None` (unset).
    pub fn max_age(mut self, max_age: usize) -> Cors {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.max_age = Some(max_age)
        }
        self
    }

    /// Set a wildcard origins
    ///
    /// If send wildcard is set and the `allowed_origins` parameter is `All`, a
    /// wildcard `Access-Control-Allow-Origin` response header is sent,
    /// rather than the request’s `Origin` header.
    ///
    /// This is the `supports credentials flag` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// This **CANNOT** be used in conjunction with `allowed_origins` set to
    /// `All` and `allow_credentials` set to `true`. Depending on the mode
    /// of usage, this will either result in an `Error::
    /// CredentialsWithWildcardOrigin` error during actix launch or runtime.
    ///
    /// Defaults to `false`.
    pub fn send_wildcard(mut self) -> Cors {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.send_wildcard = true
        }
        self
    }

    /// Allows users to make authenticated requests
    ///
    /// If true, injects the `Access-Control-Allow-Credentials` header in
    /// responses. This allows cookies and credentials to be submitted
    /// across domains.
    ///
    /// This option cannot be used in conjunction with an `allowed_origin` set
    /// to `All` and `send_wildcards` set to `true`.
    ///
    /// Defaults to `false`.
    ///
    /// Builder panics if credentials are allowed, but the Origin is set to "*".
    /// This is not allowed by W3C
    pub fn supports_credentials(mut self) -> Cors {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.supports_credentials = true
        }
        self
    }

    /// Disable `Vary` header support.
    ///
    /// When enabled the header `Vary: Origin` will be returned as per the W3
    /// implementation guidelines.
    ///
    /// Setting this header when the `Access-Control-Allow-Origin` is
    /// dynamically generated (e.g. when there is more than one allowed
    /// origin, and an Origin than '*' is returned) informs CDNs and other
    /// caches that the CORS headers are dynamic, and cannot be cached.
    ///
    /// By default `vary` header support is enabled.
    pub fn disable_vary_header(mut self) -> Cors {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.vary_header = false
        }
        self
    }

    /// Disable *preflight* request support.
    ///
    /// When enabled cors middleware automatically handles *OPTIONS* request.
    /// This is useful application level middleware.
    ///
    /// By default *preflight* support is enabled.
    pub fn disable_preflight(mut self) -> Cors {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.preflight = false
        }
        self
    }
}

fn cors<'a>(
    parts: &'a mut Option<Inner>,
    err: &Option<http::Error>,
) -> Option<&'a mut Inner> {
    if err.is_some() {
        return None;
    }
    parts.as_mut()
}

impl<S, B> IntoTransform<CorsFactory, S> for Cors
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    fn into_transform(self) -> CorsFactory {
        let mut slf = if !self.methods {
            self.allowed_methods(vec![
                Method::GET,
                Method::HEAD,
                Method::POST,
                Method::OPTIONS,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ])
        } else {
            self
        };

        if let Some(e) = slf.error.take() {
            panic!("{}", e);
        }

        let mut cors = slf.cors.take().expect("cannot reuse CorsBuilder");

        if cors.supports_credentials && cors.send_wildcard && cors.origins.is_all() {
            panic!("Credentials are allowed, but the Origin is set to \"*\"");
        }

        if let AllOrSome::Some(ref origins) = cors.origins {
            let s = origins
                .iter()
                .fold(String::new(), |s, v| format!("{}, {}", s, v));
            cors.origins_str = Some(HeaderValue::try_from(&s[2..]).unwrap());
        }

        if !slf.expose_hdrs.is_empty() {
            cors.expose_hdrs = Some(
                slf.expose_hdrs
                    .iter()
                    .fold(String::new(), |s, v| format!("{}, {}", s, v.as_str()))[2..]
                    .to_owned(),
            );
        }

        CorsFactory {
            inner: Rc::new(cors),
        }
    }
}

/// `Middleware` for Cross-origin resource sharing support
///
/// The Cors struct contains the settings for CORS requests to be validated and
/// for responses to be generated.
pub struct CorsFactory {
    inner: Rc<Inner>,
}

impl<S, B> Transform<S> for CorsFactory
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = CorsMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(CorsMiddleware {
            service,
            inner: self.inner.clone(),
        })
    }
}

/// `Middleware` for Cross-origin resource sharing support
///
/// The Cors struct contains the settings for CORS requests to be validated and
/// for responses to be generated.
#[derive(Clone)]
pub struct CorsMiddleware<S> {
    service: S,
    inner: Rc<Inner>,
}

struct Inner {
    methods: HashSet<Method>,
    origins: AllOrSome<HashSet<String>>,
    origins_str: Option<HeaderValue>,
    headers: AllOrSome<HashSet<HeaderName>>,
    expose_hdrs: Option<String>,
    max_age: Option<usize>,
    preflight: bool,
    send_wildcard: bool,
    supports_credentials: bool,
    vary_header: bool,
}

impl Inner {
    fn validate_origin(&self, req: &RequestHead) -> Result<(), CorsError> {
        if let Some(hdr) = req.headers().get(&header::ORIGIN) {
            if let Ok(origin) = hdr.to_str() {
                return match self.origins {
                    AllOrSome::All => Ok(()),
                    AllOrSome::Some(ref allowed_origins) => allowed_origins
                        .get(origin)
                        .and_then(|_| Some(()))
                        .ok_or_else(|| CorsError::OriginNotAllowed),
                };
            }
            Err(CorsError::BadOrigin)
        } else {
            match self.origins {
                AllOrSome::All => Ok(()),
                _ => Err(CorsError::MissingOrigin),
            }
        }
    }

    fn access_control_allow_origin(&self, req: &RequestHead) -> Option<HeaderValue> {
        match self.origins {
            AllOrSome::All => {
                if self.send_wildcard {
                    Some(HeaderValue::from_static("*"))
                } else if let Some(origin) = req.headers().get(&header::ORIGIN) {
                    Some(origin.clone())
                } else {
                    None
                }
            }
            AllOrSome::Some(ref origins) => {
                if let Some(origin) =
                    req.headers()
                        .get(&header::ORIGIN)
                        .filter(|o| match o.to_str() {
                            Ok(os) => origins.contains(os),
                            _ => false,
                        })
                {
                    Some(origin.clone())
                } else {
                    Some(self.origins_str.as_ref().unwrap().clone())
                }
            }
        }
    }

    fn validate_allowed_method(&self, req: &RequestHead) -> Result<(), CorsError> {
        if let Some(hdr) = req.headers().get(&header::ACCESS_CONTROL_REQUEST_METHOD) {
            if let Ok(meth) = hdr.to_str() {
                if let Ok(method) = Method::try_from(meth) {
                    return self
                        .methods
                        .get(&method)
                        .and_then(|_| Some(()))
                        .ok_or_else(|| CorsError::MethodNotAllowed);
                }
            }
            Err(CorsError::BadRequestMethod)
        } else {
            Err(CorsError::MissingRequestMethod)
        }
    }

    fn validate_allowed_headers(&self, req: &RequestHead) -> Result<(), CorsError> {
        match self.headers {
            AllOrSome::All => Ok(()),
            AllOrSome::Some(ref allowed_headers) => {
                if let Some(hdr) =
                    req.headers().get(&header::ACCESS_CONTROL_REQUEST_HEADERS)
                {
                    if let Ok(headers) = hdr.to_str() {
                        let mut hdrs = HashSet::new();
                        for hdr in headers.split(',') {
                            match HeaderName::try_from(hdr.trim()) {
                                Ok(hdr) => hdrs.insert(hdr),
                                Err(_) => return Err(CorsError::BadRequestHeaders),
                            };
                        }
                        // `Access-Control-Request-Headers` must contain 1 or more
                        // `field-name`.
                        if !hdrs.is_empty() {
                            if !hdrs.is_subset(allowed_headers) {
                                return Err(CorsError::HeadersNotAllowed);
                            }
                            return Ok(());
                        }
                    }
                    Err(CorsError::BadRequestHeaders)
                } else {
                    Ok(())
                }
            }
        }
    }
}

impl<S, B> Service for CorsMiddleware<S>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Either<
        FutureResult<Self::Response, Error>,
        Either<S::Future, Box<dyn Future<Item = Self::Response, Error = Error>>>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        if self.inner.preflight && Method::OPTIONS == *req.method() {
            if let Err(e) = self
                .inner
                .validate_origin(req.head())
                .and_then(|_| self.inner.validate_allowed_method(req.head()))
                .and_then(|_| self.inner.validate_allowed_headers(req.head()))
            {
                return Either::A(ok(req.error_response(e)));
            }

            // allowed headers
            let headers = if let Some(headers) = self.inner.headers.as_ref() {
                Some(
                    HeaderValue::try_from(
                        &headers
                            .iter()
                            .fold(String::new(), |s, v| s + "," + v.as_str())
                            .as_str()[1..],
                    )
                    .unwrap(),
                )
            } else if let Some(hdr) =
                req.headers().get(&header::ACCESS_CONTROL_REQUEST_HEADERS)
            {
                Some(hdr.clone())
            } else {
                None
            };

            let res = HttpResponse::Ok()
                .if_some(self.inner.max_age.as_ref(), |max_age, resp| {
                    let _ = resp.header(
                        header::ACCESS_CONTROL_MAX_AGE,
                        format!("{}", max_age).as_str(),
                    );
                })
                .if_some(headers, |headers, resp| {
                    let _ = resp.header(header::ACCESS_CONTROL_ALLOW_HEADERS, headers);
                })
                .if_some(
                    self.inner.access_control_allow_origin(req.head()),
                    |origin, resp| {
                        let _ = resp.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
                    },
                )
                .if_true(self.inner.supports_credentials, |resp| {
                    resp.header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true");
                })
                .header(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    &self
                        .inner
                        .methods
                        .iter()
                        .fold(String::new(), |s, v| s + "," + v.as_str())
                        .as_str()[1..],
                )
                .finish()
                .into_body();

            Either::A(ok(req.into_response(res)))
        } else if req.headers().contains_key(&header::ORIGIN) {
            // Only check requests with a origin header.
            if let Err(e) = self.inner.validate_origin(req.head()) {
                return Either::A(ok(req.error_response(e)));
            }

            let inner = self.inner.clone();

            Either::B(Either::B(Box::new(self.service.call(req).and_then(
                move |mut res| {
                    if let Some(origin) =
                        inner.access_control_allow_origin(res.request().head())
                    {
                        res.headers_mut()
                            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
                    };

                    if let Some(ref expose) = inner.expose_hdrs {
                        res.headers_mut().insert(
                            header::ACCESS_CONTROL_EXPOSE_HEADERS,
                            HeaderValue::try_from(expose.as_str()).unwrap(),
                        );
                    }
                    if inner.supports_credentials {
                        res.headers_mut().insert(
                            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                            HeaderValue::from_static("true"),
                        );
                    }
                    if inner.vary_header {
                        let value =
                            if let Some(hdr) = res.headers_mut().get(&header::VARY) {
                                let mut val: Vec<u8> =
                                    Vec::with_capacity(hdr.as_bytes().len() + 8);
                                val.extend(hdr.as_bytes());
                                val.extend(b", Origin");
                                HeaderValue::try_from(&val[..]).unwrap()
                            } else {
                                HeaderValue::from_static("Origin")
                            };
                        res.headers_mut().insert(header::VARY, value);
                    }
                    Ok(res)
                },
            ))))
        } else {
            Either::B(Either::A(self.service.call(req)))
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_service::{IntoService, Transform};
    use actix_web::test::{self, block_on, TestRequest};

    use super::*;

    impl Cors {
        fn finish<F, S, B>(self, srv: F) -> CorsMiddleware<S>
        where
            F: IntoService<S>,
            S: Service<
                    Request = ServiceRequest,
                    Response = ServiceResponse<B>,
                    Error = Error,
                > + 'static,
            S::Future: 'static,
            B: 'static,
        {
            block_on(
                IntoTransform::<CorsFactory, S>::into_transform(self)
                    .new_transform(srv.into_service()),
            )
            .unwrap()
        }
    }

    #[test]
    #[should_panic(expected = "Credentials are allowed, but the Origin is set to")]
    fn cors_validates_illegal_allow_credentials() {
        let _cors = Cors::new()
            .supports_credentials()
            .send_wildcard()
            .finish(test::ok_service());
    }

    #[test]
    fn validate_origin_allows_all_origins() {
        let mut cors = Cors::new().finish(test::ok_service());
        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn default() {
        let mut cors =
            block_on(Cors::default().new_transform(test::ok_service())).unwrap();
        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_preflight() {
        let mut cors = Cors::new()
            .send_wildcard()
            .max_age(3600)
            .allowed_methods(vec![Method::GET, Method::OPTIONS, Method::POST])
            .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
            .allowed_header(header::CONTENT_TYPE)
            .finish(test::ok_service());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "X-Not-Allowed")
            .to_srv_request();

        assert!(cors.inner.validate_allowed_method(req.head()).is_err());
        assert!(cors.inner.validate_allowed_headers(req.head()).is_err());
        let resp = test::call_service(&mut cors, req);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "put")
            .method(Method::OPTIONS)
            .to_srv_request();

        assert!(cors.inner.validate_allowed_method(req.head()).is_err());
        assert!(cors.inner.validate_allowed_headers(req.head()).is_ok());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                "AUTHORIZATION,ACCEPT",
            )
            .method(Method::OPTIONS)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"*"[..],
            resp.headers()
                .get(&header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );
        assert_eq!(
            &b"3600"[..],
            resp.headers()
                .get(&header::ACCESS_CONTROL_MAX_AGE)
                .unwrap()
                .as_bytes()
        );
        let hdr = resp
            .headers()
            .get(&header::ACCESS_CONTROL_ALLOW_HEADERS)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(hdr.contains("authorization"));
        assert!(hdr.contains("accept"));
        assert!(hdr.contains("content-type"));

        let methods = resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(methods.contains("POST"));
        assert!(methods.contains("GET"));
        assert!(methods.contains("OPTIONS"));

        Rc::get_mut(&mut cors.inner).unwrap().preflight = false;

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                "AUTHORIZATION,ACCEPT",
            )
            .method(Method::OPTIONS)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // #[test]
    // #[should_panic(expected = "MissingOrigin")]
    // fn test_validate_missing_origin() {
    //    let cors = Cors::build()
    //        .allowed_origin("https://www.example.com")
    //        .finish();
    //    let mut req = HttpRequest::default();
    //    cors.start(&req).unwrap();
    // }

    #[test]
    #[should_panic(expected = "OriginNotAllowed")]
    fn test_validate_not_allowed_origin() {
        let cors = Cors::new()
            .allowed_origin("https://www.example.com")
            .finish(test::ok_service());

        let req = TestRequest::with_header("Origin", "https://www.unknown.com")
            .method(Method::GET)
            .to_srv_request();
        cors.inner.validate_origin(req.head()).unwrap();
        cors.inner.validate_allowed_method(req.head()).unwrap();
        cors.inner.validate_allowed_headers(req.head()).unwrap();
    }

    #[test]
    fn test_validate_origin() {
        let mut cors = Cors::new()
            .allowed_origin("https://www.example.com")
            .finish(test::ok_service());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::GET)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_no_origin_response() {
        let mut cors = Cors::new().disable_preflight().finish(test::ok_service());

        let req = TestRequest::default().method(Method::GET).to_srv_request();
        let resp = test::call_service(&mut cors, req);
        assert!(resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .to_srv_request();
        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"https://www.example.com"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );
    }

    #[test]
    fn test_response() {
        let exposed_headers = vec![header::AUTHORIZATION, header::ACCEPT];
        let mut cors = Cors::new()
            .send_wildcard()
            .disable_preflight()
            .max_age(3600)
            .allowed_methods(vec![Method::GET, Method::OPTIONS, Method::POST])
            .allowed_headers(exposed_headers.clone())
            .expose_headers(exposed_headers.clone())
            .allowed_header(header::CONTENT_TYPE)
            .finish(test::ok_service());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"*"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );
        assert_eq!(
            &b"Origin"[..],
            resp.headers().get(header::VARY).unwrap().as_bytes()
        );

        {
            let headers = resp
                .headers()
                .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
                .unwrap()
                .to_str()
                .unwrap()
                .split(',')
                .map(|s| s.trim())
                .collect::<Vec<&str>>();

            for h in exposed_headers {
                assert!(headers.contains(&h.as_str()));
            }
        }

        let exposed_headers = vec![header::AUTHORIZATION, header::ACCEPT];
        let mut cors = Cors::new()
            .send_wildcard()
            .disable_preflight()
            .max_age(3600)
            .allowed_methods(vec![Method::GET, Method::OPTIONS, Method::POST])
            .allowed_headers(exposed_headers.clone())
            .expose_headers(exposed_headers.clone())
            .allowed_header(header::CONTENT_TYPE)
            .finish(|req: ServiceRequest| {
                req.into_response(
                    HttpResponse::Ok().header(header::VARY, "Accept").finish(),
                )
            });
        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .to_srv_request();
        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"Accept, Origin"[..],
            resp.headers().get(header::VARY).unwrap().as_bytes()
        );

        let mut cors = Cors::new()
            .disable_vary_header()
            .allowed_origin("https://www.example.com")
            .allowed_origin("https://www.google.com")
            .finish(test::ok_service());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .to_srv_request();
        let resp = test::call_service(&mut cors, req);

        let origins_str = resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!("https://www.example.com", origins_str);
    }

    #[test]
    fn test_multiple_origins() {
        let mut cors = Cors::new()
            .allowed_origin("https://example.com")
            .allowed_origin("https://example.org")
            .allowed_methods(vec![Method::GET])
            .finish(test::ok_service());

        let req = TestRequest::with_header("Origin", "https://example.com")
            .method(Method::GET)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"https://example.com"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );

        let req = TestRequest::with_header("Origin", "https://example.org")
            .method(Method::GET)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"https://example.org"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );
    }

    #[test]
    fn test_multiple_origins_preflight() {
        let mut cors = Cors::new()
            .allowed_origin("https://example.com")
            .allowed_origin("https://example.org")
            .allowed_methods(vec![Method::GET])
            .finish(test::ok_service());

        let req = TestRequest::with_header("Origin", "https://example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .method(Method::OPTIONS)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"https://example.com"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );

        let req = TestRequest::with_header("Origin", "https://example.org")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .method(Method::OPTIONS)
            .to_srv_request();

        let resp = test::call_service(&mut cors, req);
        assert_eq!(
            &b"https://example.org"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );
    }
}

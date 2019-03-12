//! Cross-origin resource sharing (CORS) for Actix applications
//!
//! CORS middleware could be used with application and with resource.
//! First you need to construct CORS middleware instance.
//!
//! To construct a cors:
//!
//!   1. Call [`Cors::build`](struct.Cors.html#method.build) to start building.
//!   2. Use any of the builder methods to set fields in the backend.
//!   3. Call [finish](struct.Cors.html#method.finish) to retrieve the
//!      constructed backend.
//!
//! Cors middleware could be used as parameter for `App::middleware()` or
//! `Resource::middleware()` methods. But you have to use
//! `Cors::for_app()` method to support *preflight* OPTIONS request.
//!
//!
//! # Example
//!
//! ```rust
//! # extern crate actix_web;
//! use actix_web::middleware::cors::Cors;
//! use actix_web::{http, App, HttpRequest, HttpResponse};
//!
//! fn index(mut req: HttpRequest) -> &'static str {
//!     "Hello world"
//! }
//!
//! fn main() {
//!     let app = App::new().configure(|app| {
//!         Cors::for_app(app) // <- Construct CORS middleware builder
//!             .allowed_origin("https://www.rust-lang.org/")
//!             .allowed_methods(vec!["GET", "POST"])
//!             .allowed_headers(vec![http::header::AUTHORIZATION, http::header::ACCEPT])
//!             .allowed_header(http::header::CONTENT_TYPE)
//!             .max_age(3600)
//!             .resource("/index.html", |r| {
//!                 r.method(http::Method::GET).f(|_| HttpResponse::Ok());
//!                 r.method(http::Method::HEAD).f(|_| HttpResponse::MethodNotAllowed());
//!             })
//!             .register()
//!     });
//! }
//! ```
//! In this example custom *CORS* middleware get registered for "/index.html"
//! endpoint.
//!
//! Cors middleware automatically handle *OPTIONS* preflight request.
use std::collections::HashSet;
use std::iter::FromIterator;
use std::rc::Rc;

use http::header::{self, HeaderName, HeaderValue};
use http::{self, HttpTryFrom, Method, StatusCode, Uri};

use application::App;
use error::{ResponseError, Result};
use httpmessage::HttpMessage;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Middleware, Response, Started};
use resource::Resource;
use router::ResourceDef;
use server::Request;

/// A set of errors that can occur during processing CORS
#[derive(Debug, Fail)]
pub enum CorsError {
    /// The HTTP request header `Origin` is required but was not provided
    #[fail(
        display = "The HTTP request header `Origin` is required but was not provided"
    )]
    MissingOrigin,
    /// The HTTP request header `Origin` could not be parsed correctly.
    #[fail(display = "The HTTP request header `Origin` could not be parsed correctly.")]
    BadOrigin,
    /// The request header `Access-Control-Request-Method` is required but is
    /// missing
    #[fail(
        display = "The request header `Access-Control-Request-Method` is required but is missing"
    )]
    MissingRequestMethod,
    /// The request header `Access-Control-Request-Method` has an invalid value
    #[fail(
        display = "The request header `Access-Control-Request-Method` has an invalid value"
    )]
    BadRequestMethod,
    /// The request header `Access-Control-Request-Headers`  has an invalid
    /// value
    #[fail(
        display = "The request header `Access-Control-Request-Headers`  has an invalid value"
    )]
    BadRequestHeaders,
    /// The request header `Access-Control-Request-Headers`  is required but is
    /// missing.
    #[fail(
        display = "The request header `Access-Control-Request-Headers`  is required but is
                     missing"
    )]
    MissingRequestHeaders,
    /// Origin is not allowed to make this request
    #[fail(display = "Origin is not allowed to make this request")]
    OriginNotAllowed,
    /// Requested method is not allowed
    #[fail(display = "Requested method is not allowed")]
    MethodNotAllowed,
    /// One or more headers requested are not allowed
    #[fail(display = "One or more headers requested are not allowed")]
    HeadersNotAllowed,
}

impl ResponseError for CorsError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::with_body(StatusCode::BAD_REQUEST, format!("{}", self))
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

/// `Middleware` for Cross-origin resource sharing support
///
/// The Cors struct contains the settings for CORS requests to be validated and
/// for responses to be generated.
#[derive(Clone)]
pub struct Cors {
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

impl Default for Cors {
    fn default() -> Cors {
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
                ].into_iter(),
            ),
            headers: AllOrSome::All,
            expose_hdrs: None,
            max_age: None,
            preflight: true,
            send_wildcard: false,
            supports_credentials: false,
            vary_header: true,
        };
        Cors {
            inner: Rc::new(inner),
        }
    }
}

impl Cors {
    /// Build a new CORS middleware instance
    pub fn build() -> CorsBuilder<()> {
        CorsBuilder {
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
            resources: Vec::new(),
            app: None,
        }
    }

    /// Create CorsBuilder for a specified application.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::middleware::cors::Cors;
    /// use actix_web::{http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().configure(
    ///         |app| {
    ///             Cors::for_app(app)   // <- Construct CORS builder
    ///             .allowed_origin("https://www.rust-lang.org/")
    ///             .resource("/resource", |r| {       // register resource
    ///                  r.method(http::Method::GET).f(|_| HttpResponse::Ok());
    ///             })
    ///             .register()
    ///         }, // construct CORS and return application instance
    ///     );
    /// }
    /// ```
    pub fn for_app<S: 'static>(app: App<S>) -> CorsBuilder<S> {
        CorsBuilder {
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
            resources: Vec::new(),
            app: Some(app),
        }
    }

    /// This method register cors middleware with resource and
    /// adds route for *OPTIONS* preflight requests.
    ///
    /// It is possible to register *Cors* middleware with
    /// `Resource::middleware()` method, but in that case *Cors*
    /// middleware wont be able to handle *OPTIONS* requests.
    pub fn register<S: 'static>(self, resource: &mut Resource<S>) {
        resource
            .method(Method::OPTIONS)
            .h(|_: &_| HttpResponse::Ok());
        resource.middleware(self);
    }

    fn validate_origin(&self, req: &Request) -> Result<(), CorsError> {
        if let Some(hdr) = req.headers().get(header::ORIGIN) {
            if let Ok(origin) = hdr.to_str() {
                return match self.inner.origins {
                    AllOrSome::All => Ok(()),
                    AllOrSome::Some(ref allowed_origins) => allowed_origins
                        .get(origin)
                        .and_then(|_| Some(()))
                        .ok_or_else(|| CorsError::OriginNotAllowed),
                };
            }
            Err(CorsError::BadOrigin)
        } else {
            return match self.inner.origins {
                AllOrSome::All => Ok(()),
                _ => Err(CorsError::MissingOrigin),
            };
        }
    }

    fn access_control_allow_origin(&self, req: &Request) -> Option<HeaderValue> {
        match self.inner.origins {
            AllOrSome::All => {
                if self.inner.send_wildcard {
                    Some(HeaderValue::from_static("*"))
                } else if let Some(origin) = req.headers().get(header::ORIGIN) {
                    Some(origin.clone())
                } else {
                    None
                }
            }
            AllOrSome::Some(ref origins) => {
                if let Some(origin) = req.headers().get(header::ORIGIN).filter(|o| {
                        match o.to_str() {
                            Ok(os) => origins.contains(os),
                            _ => false
                        }
                    }) {
                    Some(origin.clone())
                } else {
                    Some(self.inner.origins_str.as_ref().unwrap().clone())
                }
            }
        }
    }

    fn validate_allowed_method(&self, req: &Request) -> Result<(), CorsError> {
        if let Some(hdr) = req.headers().get(header::ACCESS_CONTROL_REQUEST_METHOD) {
            if let Ok(meth) = hdr.to_str() {
                if let Ok(method) = Method::try_from(meth) {
                    return self
                        .inner
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

    fn validate_allowed_headers(&self, req: &Request) -> Result<(), CorsError> {
        match self.inner.headers {
            AllOrSome::All => Ok(()),
            AllOrSome::Some(ref allowed_headers) => {
                if let Some(hdr) =
                    req.headers().get(header::ACCESS_CONTROL_REQUEST_HEADERS)
                {
                    if let Ok(headers) = hdr.to_str() {
                        let mut hdrs = HashSet::new();
                        for hdr in headers.split(',') {
                            match HeaderName::try_from(hdr.trim()) {
                                Ok(hdr) => hdrs.insert(hdr),
                                Err(_) => return Err(CorsError::BadRequestHeaders),
                            };
                        }

                        if !hdrs.is_empty() && !hdrs.is_subset(allowed_headers) {
                            return Err(CorsError::HeadersNotAllowed);
                        }
                        return Ok(());
                    }
                    Err(CorsError::BadRequestHeaders)
                } else {
                    Err(CorsError::MissingRequestHeaders)
                }
            }
        }
    }
}

impl<S> Middleware<S> for Cors {
    fn start(&self, req: &HttpRequest<S>) -> Result<Started> {
        if self.inner.preflight && Method::OPTIONS == *req.method() {
            self.validate_origin(req)?;
            self.validate_allowed_method(&req)?;
            self.validate_allowed_headers(&req)?;

            // allowed headers
            let headers = if let Some(headers) = self.inner.headers.as_ref() {
                Some(
                    HeaderValue::try_from(
                        &headers
                            .iter()
                            .fold(String::new(), |s, v| s + "," + v.as_str())
                            .as_str()[1..],
                    ).unwrap(),
                )
            } else if let Some(hdr) =
                req.headers().get(header::ACCESS_CONTROL_REQUEST_HEADERS)
            {
                Some(hdr.clone())
            } else {
                None
            };

            Ok(Started::Response(
                HttpResponse::Ok()
                    .if_some(self.inner.max_age.as_ref(), |max_age, resp| {
                        let _ = resp.header(
                            header::ACCESS_CONTROL_MAX_AGE,
                            format!("{}", max_age).as_str(),
                        );
                    }).if_some(headers, |headers, resp| {
                        let _ =
                            resp.header(header::ACCESS_CONTROL_ALLOW_HEADERS, headers);
                    }).if_some(self.access_control_allow_origin(&req), |origin, resp| {
                        let _ =
                            resp.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
                    }).if_true(self.inner.supports_credentials, |resp| {
                        resp.header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true");
                    }).header(
                        header::ACCESS_CONTROL_ALLOW_METHODS,
                        &self
                            .inner
                            .methods
                            .iter()
                            .fold(String::new(), |s, v| s + "," + v.as_str())
                            .as_str()[1..],
                    ).finish(),
            ))
        } else {
            // Only check requests with a origin header.
            if req.headers().contains_key(header::ORIGIN) {
                self.validate_origin(req)?;
            }

            Ok(Started::Done)
        }
    }

    fn response(
        &self, req: &HttpRequest<S>, mut resp: HttpResponse,
    ) -> Result<Response> {

        if let Some(origin) = self.access_control_allow_origin(req) {
            resp.headers_mut()
                .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        };

        if let Some(ref expose) = self.inner.expose_hdrs {
            resp.headers_mut().insert(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                HeaderValue::try_from(expose.as_str()).unwrap(),
            );
        }
        if self.inner.supports_credentials {
            resp.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }
        if self.inner.vary_header {
            let value = if let Some(hdr) = resp.headers_mut().get(header::VARY) {
                let mut val: Vec<u8> = Vec::with_capacity(hdr.as_bytes().len() + 8);
                val.extend(hdr.as_bytes());
                val.extend(b", Origin");
                HeaderValue::try_from(&val[..]).unwrap()
            } else {
                HeaderValue::from_static("Origin")
            };
            resp.headers_mut().insert(header::VARY, value);
        }
        Ok(Response::Done(resp))
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
/// # extern crate http;
/// # extern crate actix_web;
/// use actix_web::middleware::cors;
/// use http::header;
///
/// # fn main() {
/// let cors = cors::Cors::build()
///     .allowed_origin("https://www.rust-lang.org/")
///     .allowed_methods(vec!["GET", "POST"])
///     .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
///     .allowed_header(header::CONTENT_TYPE)
///     .max_age(3600)
///     .finish();
/// # }
/// ```
pub struct CorsBuilder<S = ()> {
    cors: Option<Inner>,
    methods: bool,
    error: Option<http::Error>,
    expose_hdrs: HashSet<HeaderName>,
    resources: Vec<Resource<S>>,
    app: Option<App<S>>,
}

fn cors<'a>(
    parts: &'a mut Option<Inner>, err: &Option<http::Error>,
) -> Option<&'a mut Inner> {
    if err.is_some() {
        return None;
    }
    parts.as_mut()
}

impl<S: 'static> CorsBuilder<S> {
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
    pub fn allowed_origin(&mut self, origin: &str) -> &mut CorsBuilder<S> {
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
    pub fn allowed_methods<U, M>(&mut self, methods: U) -> &mut CorsBuilder<S>
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
    pub fn allowed_header<H>(&mut self, header: H) -> &mut CorsBuilder<S>
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
    pub fn allowed_headers<U, H>(&mut self, headers: U) -> &mut CorsBuilder<S>
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
    pub fn expose_headers<U, H>(&mut self, headers: U) -> &mut CorsBuilder<S>
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
    pub fn max_age(&mut self, max_age: usize) -> &mut CorsBuilder<S> {
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
    pub fn send_wildcard(&mut self) -> &mut CorsBuilder<S> {
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
    pub fn supports_credentials(&mut self) -> &mut CorsBuilder<S> {
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
    pub fn disable_vary_header(&mut self) -> &mut CorsBuilder<S> {
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
    pub fn disable_preflight(&mut self) -> &mut CorsBuilder<S> {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.preflight = false
        }
        self
    }

    /// Configure resource for a specific path.
    ///
    /// This is similar to a `App::resource()` method. Except, cors middleware
    /// get registered for the resource.
    ///
    /// ```rust
    /// # extern crate actix_web;
    /// use actix_web::middleware::cors::Cors;
    /// use actix_web::{http, App, HttpResponse};
    ///
    /// fn main() {
    ///     let app = App::new().configure(
    ///         |app| {
    ///             Cors::for_app(app)   // <- Construct CORS builder
    ///             .allowed_origin("https://www.rust-lang.org/")
    ///             .allowed_methods(vec!["GET", "POST"])
    ///             .allowed_header(http::header::CONTENT_TYPE)
    ///             .max_age(3600)
    ///             .resource("/resource1", |r| {       // register resource
    ///                  r.method(http::Method::GET).f(|_| HttpResponse::Ok());
    ///             })
    ///             .resource("/resource2", |r| {       // register another resource
    ///                  r.method(http::Method::HEAD)
    ///                      .f(|_| HttpResponse::MethodNotAllowed());
    ///             })
    ///             .register()
    ///         }, // construct CORS and return application instance
    ///     );
    /// }
    /// ```
    pub fn resource<F, R>(&mut self, path: &str, f: F) -> &mut CorsBuilder<S>
    where
        F: FnOnce(&mut Resource<S>) -> R + 'static,
    {
        // add resource handler
        let mut resource = Resource::new(ResourceDef::new(path));
        f(&mut resource);

        self.resources.push(resource);
        self
    }

    fn construct(&mut self) -> Cors {
        if !self.methods {
            self.allowed_methods(vec![
                Method::GET,
                Method::HEAD,
                Method::POST,
                Method::OPTIONS,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ]);
        }

        if let Some(e) = self.error.take() {
            panic!("{}", e);
        }

        let mut cors = self.cors.take().expect("cannot reuse CorsBuilder");

        if cors.supports_credentials && cors.send_wildcard && cors.origins.is_all() {
            panic!("Credentials are allowed, but the Origin is set to \"*\"");
        }

        if let AllOrSome::Some(ref origins) = cors.origins {
            let s = origins
                .iter()
                .fold(String::new(), |s, v| format!("{}, {}", s, v));
            cors.origins_str = Some(HeaderValue::try_from(&s[2..]).unwrap());
        }

        if !self.expose_hdrs.is_empty() {
            cors.expose_hdrs = Some(
                self.expose_hdrs
                    .iter()
                    .fold(String::new(), |s, v| format!("{}, {}", s, v.as_str()))[2..]
                    .to_owned(),
            );
        }
        Cors {
            inner: Rc::new(cors),
        }
    }

    /// Finishes building and returns the built `Cors` instance.
    ///
    /// This method panics in case of any configuration error.
    pub fn finish(&mut self) -> Cors {
        if !self.resources.is_empty() {
            panic!(
                "CorsBuilder::resource() was used,
                    to construct CORS `.register(app)` method should be used"
            );
        }
        self.construct()
    }

    /// Finishes building Cors middleware and register middleware for
    /// application
    ///
    /// This method panics in case of any configuration error or if non of
    /// resources are registered.
    pub fn register(&mut self) -> App<S> {
        if self.resources.is_empty() {
            panic!("No resources are registered.");
        }

        let cors = self.construct();
        let mut app = self
            .app
            .take()
            .expect("CorsBuilder has to be constructed with Cors::for_app(app)");

        // register resources
        for mut resource in self.resources.drain(..) {
            cors.clone().register(&mut resource);
            app.register_resource(resource);
        }

        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test::{self, TestRequest};

    impl Started {
        fn is_done(&self) -> bool {
            match *self {
                Started::Done => true,
                _ => false,
            }
        }
        fn response(self) -> HttpResponse {
            match self {
                Started::Response(resp) => resp,
                _ => panic!(),
            }
        }
    }
    impl Response {
        fn response(self) -> HttpResponse {
            match self {
                Response::Done(resp) => resp,
                _ => panic!(),
            }
        }
    }

    #[test]
    #[should_panic(expected = "Credentials are allowed, but the Origin is set to")]
    fn cors_validates_illegal_allow_credentials() {
        Cors::build()
            .supports_credentials()
            .send_wildcard()
            .finish();
    }

    #[test]
    #[should_panic(expected = "No resources are registered")]
    fn no_resource() {
        Cors::build()
            .supports_credentials()
            .send_wildcard()
            .register();
    }

    #[test]
    #[should_panic(expected = "Cors::for_app(app)")]
    fn no_resource2() {
        Cors::build()
            .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
            .register();
    }

    #[test]
    fn validate_origin_allows_all_origins() {
        let cors = Cors::default();
        let req = TestRequest::with_header("Origin", "https://www.example.com").finish();

        assert!(cors.start(&req).ok().unwrap().is_done())
    }

    #[test]
    fn test_preflight() {
        let mut cors = Cors::build()
            .send_wildcard()
            .max_age(3600)
            .allowed_methods(vec![Method::GET, Method::OPTIONS, Method::POST])
            .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
            .allowed_header(header::CONTENT_TYPE)
            .finish();

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .finish();

        assert!(cors.start(&req).is_err());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "put")
            .method(Method::OPTIONS)
            .finish();

        assert!(cors.start(&req).is_err());

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                "AUTHORIZATION,ACCEPT",
            ).method(Method::OPTIONS)
            .finish();

        let resp = cors.start(&req).unwrap().response();
        assert_eq!(
            &b"*"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );
        assert_eq!(
            &b"3600"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_MAX_AGE)
                .unwrap()
                .as_bytes()
        );
        //assert_eq!(
        //    &b"authorization,accept,content-type"[..],
        // resp.headers().get(header::ACCESS_CONTROL_ALLOW_HEADERS).unwrap().
        // as_bytes()); assert_eq!(
        //    &b"POST,GET,OPTIONS"[..],
        // resp.headers().get(header::ACCESS_CONTROL_ALLOW_METHODS).unwrap().
        // as_bytes());

        Rc::get_mut(&mut cors.inner).unwrap().preflight = false;
        assert!(cors.start(&req).unwrap().is_done());
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
        let cors = Cors::build()
            .allowed_origin("https://www.example.com")
            .finish();

        let req = TestRequest::with_header("Origin", "https://www.unknown.com")
            .method(Method::GET)
            .finish();
        cors.start(&req).unwrap();
    }

    #[test]
    fn test_validate_origin() {
        let cors = Cors::build()
            .allowed_origin("https://www.example.com")
            .finish();

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::GET)
            .finish();

        assert!(cors.start(&req).unwrap().is_done());
    }

    #[test]
    fn test_no_origin_response() {
        let cors = Cors::build().finish();

        let req = TestRequest::default().method(Method::GET).finish();
        let resp: HttpResponse = HttpResponse::Ok().into();
        let resp = cors.response(&req, resp).unwrap().response();
        assert!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .finish();
        let resp = cors.response(&req, resp).unwrap().response();
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
        let cors = Cors::build()
            .send_wildcard()
            .disable_preflight()
            .max_age(3600)
            .allowed_methods(vec![Method::GET, Method::OPTIONS, Method::POST])
            .allowed_headers(exposed_headers.clone())
            .expose_headers(exposed_headers.clone())
            .allowed_header(header::CONTENT_TYPE)
            .finish();

        let req = TestRequest::with_header("Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .finish();

        let resp: HttpResponse = HttpResponse::Ok().into();
        let resp = cors.response(&req, resp).unwrap().response();
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

        let resp: HttpResponse =
            HttpResponse::Ok().header(header::VARY, "Accept").finish();
        let resp = cors.response(&req, resp).unwrap().response();
        assert_eq!(
            &b"Accept, Origin"[..],
            resp.headers().get(header::VARY).unwrap().as_bytes()
        );

        let cors = Cors::build()
            .disable_vary_header()
            .allowed_origin("https://www.example.com")
            .allowed_origin("https://www.google.com")
            .finish();
        let resp: HttpResponse = HttpResponse::Ok().into();
        let resp = cors.response(&req, resp).unwrap().response();

        let origins_str = resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap()
            .to_str()
            .unwrap();

        assert_eq!(
            "https://www.example.com",
            origins_str
        );
    }

    #[test]
    fn cors_resource() {
        let mut srv = test::TestServer::with_factory(|| {
            App::new().configure(|app| {
                Cors::for_app(app)
                    .allowed_origin("https://www.example.com")
                    .resource("/test", |r| r.f(|_| HttpResponse::Ok()))
                    .register()
            })
        });

        let request = srv
            .get()
            .uri(srv.url("/test"))
            .header("ORIGIN", "https://www.example2.com")
            .finish()
            .unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let request = srv.get().uri(srv.url("/test")).finish().unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let request = srv
            .get()
            .uri(srv.url("/test"))
            .header("ORIGIN", "https://www.example.com")
            .finish()
            .unwrap();
        let response = srv.execute(request.send()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_multiple_origins() {
        let cors = Cors::build()
            .allowed_origin("https://example.com")
            .allowed_origin("https://example.org")
            .allowed_methods(vec![Method::GET])
            .finish();


        let req = TestRequest::with_header("Origin", "https://example.com")
            .method(Method::GET)
            .finish();
        let resp: HttpResponse = HttpResponse::Ok().into();

        let resp = cors.response(&req, resp).unwrap().response();
        assert_eq!(
            &b"https://example.com"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );

        let req = TestRequest::with_header("Origin", "https://example.org")
            .method(Method::GET)
            .finish();
        let resp: HttpResponse = HttpResponse::Ok().into();

        let resp = cors.response(&req, resp).unwrap().response();
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
        let cors = Cors::build()
            .allowed_origin("https://example.com")
            .allowed_origin("https://example.org")
            .allowed_methods(vec![Method::GET])
            .finish();


        let req = TestRequest::with_header("Origin", "https://example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .method(Method::OPTIONS)
            .finish();

        let resp = cors.start(&req).ok().unwrap().response();
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
            .finish();

        let resp = cors.start(&req).ok().unwrap().response();
        assert_eq!(
            &b"https://example.org"[..],
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap()
                .as_bytes()
        );
    }
}

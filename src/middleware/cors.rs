//! Cross-origin resource sharing (CORS) for Actix applications
//!
//! CORS middleware could be used with application and with resource.
//! First you need to construct CORS middleware instance.
//!
//! To construct a cors:
//!
//!   1. Call [`Cors::build`](struct.Cors.html#method.build) to start building.
//!   2. Use any of the builder methods to set fields in the backend.
//!   3. Call [finish](struct.Cors.html#method.finish) to retrieve the constructed backend.
//!
//! This constructed middleware could be used as parameter for `Application::middleware()` or
//! `Resource::middleware()` methods.
//!
//! # Example
//!
//! ```rust
//! # extern crate http;
//! # extern crate actix_web;
//! # use actix_web::*;
//! use http::header;
//! use actix_web::middleware::cors;
//!
//! fn index(mut req: HttpRequest) -> &'static str {
//!    "Hello world"
//! }
//!
//! fn main() {
//!     let app = Application::new()
//!         .resource("/index.html", |r| {
//!              r.middleware(cors::Cors::build()                   // <- Register CORS middleware
//!                  .allowed_origin("https://www.rust-lang.org/")
//!                  .allowed_methods(vec!["GET", "POST"])
//!                  .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
//!                  .allowed_header(header::CONTENT_TYPE)
//!                  .max_age(3600)
//!                  .finish().expect("Can not create CORS middleware"));
//!              r.method(Method::GET).f(|_| httpcodes::HTTPOk);
//!              r.method(Method::HEAD).f(|_| httpcodes::HTTPMethodNotAllowed);
//!         })
//!         .finish();
//! }
//! ```
//! In this example custom *CORS* middleware get registered for "/index.html" endpoint.
//!
//! Cors middleware automatically handle *OPTIONS* preflight request.
use std::collections::HashSet;
use std::iter::FromIterator;

use http::{self, Method, HttpTryFrom, Uri};
use http::header::{self, HeaderName, HeaderValue};

use error::{Result, ResponseError};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use httpcodes::{HTTPOk, HTTPBadRequest};
use middleware::{Middleware, Response, Started};

/// A set of errors that can occur during processing CORS
#[derive(Debug, Fail)]
pub enum CorsError {
    /// The HTTP request header `Origin` is required but was not provided
    #[fail(display="The HTTP request header `Origin` is required but was not provided")]
    MissingOrigin,
    /// The HTTP request header `Origin` could not be parsed correctly.
    #[fail(display="The HTTP request header `Origin` could not be parsed correctly.")]
    BadOrigin,
    /// The request header `Access-Control-Request-Method` is required but is missing
    #[fail(display="The request header `Access-Control-Request-Method` is required but is missing")]
    MissingRequestMethod,
    /// The request header `Access-Control-Request-Method` has an invalid value
    #[fail(display="The request header `Access-Control-Request-Method` has an invalid value")]
    BadRequestMethod,
    /// The request header `Access-Control-Request-Headers`  has an invalid value
    #[fail(display="The request header `Access-Control-Request-Headers`  has an invalid value")]
    BadRequestHeaders,
    /// The request header `Access-Control-Request-Headers`  is required but is missing.
    #[fail(display="The request header `Access-Control-Request-Headers`  is required but is
                     missing")]
    MissingRequestHeaders,
    /// Origin is not allowed to make this request
    #[fail(display="Origin is not allowed to make this request")]
    OriginNotAllowed,
    /// Requested method is not allowed
    #[fail(display="Requested method is not allowed")]
    MethodNotAllowed,
    /// One or more headers requested are not allowed
    #[fail(display="One or more headers requested are not allowed")]
    HeadersNotAllowed,
}

/// A set of errors that can occur during building CORS middleware
#[derive(Debug, Fail)]
pub enum CorsBuilderError {
    #[fail(display="Parse error: {}", _0)]
    ParseError(http::Error),
    /// Credentials are allowed, but the Origin is set to "*". This is not allowed by W3C
    ///
    /// This is a misconfiguration. Check the docuemntation for `Cors`.
    #[fail(display="Credentials are allowed, but the Origin is set to \"*\"")]
    CredentialsWithWildcardOrigin,
}


impl ResponseError for CorsError {

    fn error_response(&self) -> HttpResponse {
        HTTPBadRequest.into()
    }
}

/// An enum signifying that some of type T is allowed, or `All` (everything is allowed).
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
pub struct Cors {
    methods: HashSet<Method>,
    origins: AllOrSome<HashSet<Uri>>,
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
        Cors {
            origins: AllOrSome::default(),
            origins_str: None,
            methods: HashSet::from_iter(
                vec![Method::GET, Method::HEAD,
                     Method::POST, Method::OPTIONS, Method::PUT,
                     Method::PATCH, Method::DELETE].into_iter()),
            headers: AllOrSome::All,
            expose_hdrs: None,
            max_age: None,
            preflight: true,
            send_wildcard: false,
            supports_credentials: false,
            vary_header: true,
        }
    }
}

impl Cors {
    pub fn build() -> CorsBuilder {
        CorsBuilder {
            cors: Some(Cors {
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

    fn validate_origin<S>(&self, req: &mut HttpRequest<S>) -> Result<(), CorsError> {
        if let Some(hdr) = req.headers().get(header::ORIGIN) {
            if let Ok(origin) = hdr.to_str() {
                if let Ok(uri) = Uri::try_from(origin) {
                    return match self.origins {
                        AllOrSome::All => Ok(()),
                        AllOrSome::Some(ref allowed_origins) => {
                            allowed_origins
                                .get(&uri)
                                .and_then(|_| Some(()))
                                .ok_or_else(|| CorsError::OriginNotAllowed)
                        }
                    };
                }
            }
            Err(CorsError::BadOrigin)
        } else {
            Ok(())
        }
    }

    fn validate_allowed_method<S>(&self, req: &mut HttpRequest<S>) -> Result<(), CorsError> {
        if let Some(hdr) = req.headers().get(header::ACCESS_CONTROL_REQUEST_METHOD) {
            if let Ok(meth) = hdr.to_str() {
                if let Ok(method) = Method::try_from(meth) {
                    return self.methods.get(&method)
                        .and_then(|_| Some(()))
                        .ok_or_else(|| CorsError::MethodNotAllowed);
                }
            }
            Err(CorsError::BadRequestMethod)
        } else {
            Err(CorsError::MissingRequestMethod)
        }
    }

    fn validate_allowed_headers<S>(&self, req: &mut HttpRequest<S>) -> Result<(), CorsError> {
        if let Some(hdr) = req.headers().get(header::ACCESS_CONTROL_REQUEST_HEADERS) {
            if let Ok(headers) = hdr.to_str() {
                match self.headers {
                    AllOrSome::All => return Ok(()),
                    AllOrSome::Some(ref allowed_headers) => {
                        let mut hdrs = HashSet::new();
                        for hdr in headers.split(',') {
                            match HeaderName::try_from(hdr.trim()) {
                                Ok(hdr) => hdrs.insert(hdr),
                                Err(_) => return Err(CorsError::BadRequestHeaders)
                            };
                        }

                        if !hdrs.is_empty() && !hdrs.is_subset(allowed_headers) {
                            return Err(CorsError::HeadersNotAllowed)
                        }
                        return Ok(())
                    }
                }
            }
            Err(CorsError::BadRequestHeaders)
        } else {
            Err(CorsError::MissingRequestHeaders)
        }
    }
}

impl<S> Middleware<S> for Cors {

    fn start(&self, req: &mut HttpRequest<S>) -> Result<Started> {
        if self.preflight && Method::OPTIONS == *req.method() {
            self.validate_origin(req)?;
            self.validate_allowed_method(req)?;
            self.validate_allowed_headers(req)?;

            Ok(Started::Response(
                HTTPOk.build()
                    .if_some(self.max_age.as_ref(), |max_age, resp| {
                        let _ = resp.header(
                            header::ACCESS_CONTROL_MAX_AGE, format!("{}", max_age).as_str());})
                    .if_some(self.headers.as_ref(), |headers, resp| {
                        let _ = resp.header(
                            header::ACCESS_CONTROL_ALLOW_HEADERS,
                            &headers.iter().fold(
                                String::new(), |s, v| s + "," + v.as_str()).as_str()[1..]);})
                    .if_true(self.origins.is_all(), |resp| {
                        if self.send_wildcard {
                            resp.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
                        } else {
                            let origin = req.headers().get(header::ORIGIN).unwrap();
                            resp.header(
                                header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
                        }
                    })
                    .if_true(self.origins.is_some(), |resp| {
                        resp.header(
                            header::ACCESS_CONTROL_ALLOW_ORIGIN,
                            self.origins_str.as_ref().unwrap().clone());
                    })
                    .if_true(self.supports_credentials, |resp| {
                        resp.header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true");
                    })
                    .header(
                        header::ACCESS_CONTROL_ALLOW_METHODS,
                        &self.methods.iter().fold(
                            String::new(), |s, v| s + "," + v.as_str()).as_str()[1..])
                    .finish()
                    .unwrap()))
        } else {
            self.validate_origin(req)?;

            Ok(Started::Done)
        }
    }

    fn response(&self, req: &mut HttpRequest<S>, mut resp: HttpResponse) -> Result<Response> {
        match self.origins {
            AllOrSome::All => {
                if self.send_wildcard {
                    resp.headers_mut().insert(
                        header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
                } else {
                    let origin = req.headers().get(header::ORIGIN).unwrap();
                    resp.headers_mut().insert(
                        header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
                }
            }
            AllOrSome::Some(_) => {
                resp.headers_mut().insert(
                    header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    self.origins_str.as_ref().unwrap().clone());
            }
        }

        if let Some(ref expose) = self.expose_hdrs {
            resp.headers_mut().insert(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                HeaderValue::try_from(expose.as_str()).unwrap());
        }
        if self.supports_credentials {
            resp.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HeaderValue::from_static("true"));
        }
        if self.vary_header {
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

/// Structure that follows the builder pattern for building `Cors` middleware structs.
///
/// To construct a cors:
///
///   1. Call [`Cors::build`](struct.Cors.html#method.build) to start building.
///   2. Use any of the builder methods to set fields in the backend.
///   3. Call [finish](struct.Cors.html#method.finish) to retrieve the constructed backend.
///
/// # Example
///
/// ```rust
/// # extern crate http;
/// # extern crate actix_web;
/// use http::header;
/// use actix_web::middleware::cors;
///
/// # fn main() {
/// let cors = cors::Cors::build()
///     .allowed_origin("https://www.rust-lang.org/")
///     .allowed_methods(vec!["GET", "POST"])
///     .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
///     .allowed_header(header::CONTENT_TYPE)
///     .max_age(3600)
///     .finish().unwrap();
/// # }
/// ```
pub struct CorsBuilder {
    cors: Option<Cors>,
    methods: bool,
    error: Option<http::Error>,
    expose_hdrs: HashSet<HeaderName>,
}

fn cors<'a>(parts: &'a mut Option<Cors>, err: &Option<http::Error>) -> Option<&'a mut Cors> {
    if err.is_some() {
        return None
    }
    parts.as_mut()
}

impl CorsBuilder {

    /// Add an origin that are allowed to make requests.
    /// Will be verified against the `Origin` request header.
    ///
    /// When `All` is set, and `send_wildcard` is set, "*" will be sent in
    /// the `Access-Control-Allow-Origin` response header. Otherwise, the client's `Origin` request
    /// header will be echoed back in the `Access-Control-Allow-Origin` response header.
    ///
    /// When `Some` is set, the client's `Origin` request header will be checked in a
    /// case-sensitive manner.
    ///
    /// This is the `list of origins` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// Defaults to `All`.
    /// ```
    pub fn allowed_origin<U>(&mut self, origin: U) -> &mut CorsBuilder
        where Uri: HttpTryFrom<U>
    {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            match Uri::try_from(origin) {
                Ok(uri) => {
                    if cors.origins.is_all() {
                        cors.origins = AllOrSome::Some(HashSet::new());
                    }
                    if let AllOrSome::Some(ref mut origins) = cors.origins {
                        origins.insert(uri);
                    }
                }
                Err(e) => {
                    self.error = Some(e.into());
                }
            }
        }
        self
    }

    /// Set a list of methods which the allowed origins are allowed to access for
    /// requests.
    ///
    /// This is the `list of methods` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// Defaults to `[GET, HEAD, POST, OPTIONS, PUT, PATCH, DELETE]`
    pub fn allowed_methods<U, M>(&mut self, methods: U) -> &mut CorsBuilder
        where U: IntoIterator<Item=M>, Method: HttpTryFrom<M>
    {
        self.methods = true;
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            for m in methods {
                match Method::try_from(m) {
                    Ok(method) => {
                        cors.methods.insert(method);
                    },
                    Err(e) => {
                        self.error = Some(e.into());
                        break
                    }
                }
            };
        }
        self
    }

    /// Set an allowed header
    pub fn allowed_header<H>(&mut self, header: H) -> &mut CorsBuilder
        where HeaderName: HttpTryFrom<H>
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
    /// If `All` is set, whatever is requested by the client in `Access-Control-Request-Headers`
    /// will be echoed back in the `Access-Control-Allow-Headers` header.
    ///
    /// This is the `list of headers` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// Defaults to `All`.
    pub fn allowed_headers<U, H>(&mut self, headers: U) -> &mut CorsBuilder
        where U: IntoIterator<Item=H>, HeaderName: HttpTryFrom<H>
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
                        break
                    }
                }
            };
        }
        self
    }

    /// Set a list of headers which are safe to expose to the API of a CORS API specification.
    /// This corresponds to the `Access-Control-Expose-Headers` responde header.
    ///
    /// This is the `list of exposed headers` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// This defaults to an empty set.
    pub fn expose_headers<U, H>(&mut self, headers: U) -> &mut CorsBuilder
        where U: IntoIterator<Item=H>, HeaderName: HttpTryFrom<H>
    {
        for h in headers {
            match HeaderName::try_from(h) {
                Ok(method) => {
                    self.expose_hdrs.insert(method);
                },
                Err(e) => {
                    self.error = Some(e.into());
                    break
                }
            }
        }
        self
    }

    /// Set a maximum time for which this CORS request maybe cached.
    /// This value is set as the `Access-Control-Max-Age` header.
    ///
    /// This defaults to `None` (unset).
    pub fn max_age(&mut self, max_age: usize) -> &mut CorsBuilder {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.max_age = Some(max_age)
        }
        self
    }

    /// Set a wildcard origins
    ///
    /// If send widlcard is set and the `allowed_origins` parameter is `All`, a wildcard
    /// `Access-Control-Allow-Origin` response header is sent, rather than the request’s
    /// `Origin` header.
    ///
    /// This is the `supports credentials flag` in the
    /// [Resource Processing Model](https://www.w3.org/TR/cors/#resource-processing-model).
    ///
    /// This **CANNOT** be used in conjunction with `allowed_origins` set to `All` and
    /// `allow_credentials` set to `true`. Depending on the mode of usage, this will either result
    /// in an `Error::CredentialsWithWildcardOrigin` error during actix launch or runtime.
    ///
    /// Defaults to `false`.
    #[cfg_attr(feature = "serialization", serde(default))]
    pub fn send_wildcard(&mut self) -> &mut CorsBuilder {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.send_wildcard = true
        }
        self
    }

    /// Allows users to make authenticated requests
    ///
    /// If true, injects the `Access-Control-Allow-Credentials` header in responses.
    /// This allows cookies and credentials to be submitted across domains.
    ///
    /// This option cannot be used in conjuction with an `allowed_origin` set to `All`
    /// and `send_wildcards` set to `true`.
    ///
    /// Defaults to `false`.
    pub fn supports_credentials(&mut self) -> &mut CorsBuilder {
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
    pub fn disable_vary_header(&mut self) -> &mut CorsBuilder {
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
    pub fn disable_preflight(&mut self) -> &mut CorsBuilder {
        if let Some(cors) = cors(&mut self.cors, &self.error) {
            cors.preflight = false
        }
        self
    }

    /// Finishes building and returns the built `Cors` instance.
    pub fn finish(&mut self) -> Result<Cors, CorsBuilderError> {
        if !self.methods {
            self.allowed_methods(vec![Method::GET, Method::HEAD,
                                    Method::POST, Method::OPTIONS, Method::PUT,
                                    Method::PATCH, Method::DELETE]);
        }

        if let Some(e) = self.error.take() {
            return Err(CorsBuilderError::ParseError(e))
        }

        let mut cors = self.cors.take().expect("cannot reuse CorsBuilder");

        if cors.supports_credentials && cors.send_wildcard && cors.origins.is_all() {
            return Err(CorsBuilderError::CredentialsWithWildcardOrigin)
        }

        if let AllOrSome::Some(ref origins) = cors.origins {
            let s = origins.iter().fold(String::new(), |s, v| s + &format!("{}", v));
            cors.origins_str = Some(HeaderValue::try_from(s.as_str()).unwrap());
        }

        if !self.expose_hdrs.is_empty() {
            cors.expose_hdrs = Some(
                self.expose_hdrs.iter().fold(
                    String::new(), |s, v| s + v.as_str())[1..].to_owned());
        }
        Ok(cors)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use test::TestRequest;

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

    #[test]
    #[should_panic(expected = "CredentialsWithWildcardOrigin")]
    fn cors_validates_illegal_allow_credentials() {
        Cors::build()
            .supports_credentials()
            .send_wildcard()
            .finish()
            .unwrap();
    }

    #[test]
    fn validate_origin_allows_all_origins() {
        let cors = Cors::default();
        let mut req = TestRequest::with_header(
            "Origin", "https://www.example.com").finish();

        assert!(cors.start(&mut req).ok().unwrap().is_done())
    }

    #[test]
    fn test_preflight() {
        let mut cors = Cors::build()
            .send_wildcard()
            .max_age(3600)
            .allowed_methods(vec![Method::GET, Method::OPTIONS, Method::POST])
            .allowed_headers(vec![header::AUTHORIZATION, header::ACCEPT])
            .allowed_header(header::CONTENT_TYPE)
            .finish().unwrap();

        let mut req = TestRequest::with_header(
            "Origin", "https://www.example.com")
            .method(Method::OPTIONS)
            .finish();

        assert!(cors.start(&mut req).is_err());

        let mut req = TestRequest::with_header("Origin", "https://www.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "put")
            .method(Method::OPTIONS)
            .finish();

        assert!(cors.start(&mut req).is_err());

        let mut req = TestRequest::with_header("Origin", "https://www.example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "AUTHORIZATION,ACCEPT")
            .method(Method::OPTIONS)
            .finish();

        let resp = cors.start(&mut req).unwrap().response();
        assert_eq!(
            &b"*"[..],
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap().as_bytes());
        assert_eq!(
            &b"3600"[..],
            resp.headers().get(header::ACCESS_CONTROL_MAX_AGE).unwrap().as_bytes());
        //assert_eq!(
        //    &b"authorization,accept,content-type"[..],
        //    resp.headers().get(header::ACCESS_CONTROL_ALLOW_HEADERS).unwrap().as_bytes());
        //assert_eq!(
        //    &b"POST,GET,OPTIONS"[..],
        //    resp.headers().get(header::ACCESS_CONTROL_ALLOW_METHODS).unwrap().as_bytes());

        cors.preflight = false;
        assert!(cors.start(&mut req).unwrap().is_done());
    }

    #[test]
    fn test_validate_origin() {
        let cors = Cors::build()
            .allowed_origin("http://www.example.com").finish().unwrap();

        let mut req = TestRequest::with_header(
            "Origin", "https://www.unknown.com")
            .method(Method::GET)
            .finish();

        assert!(cors.start(&mut req).is_err());
    }
}

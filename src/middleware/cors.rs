//! Cross-origin resource sharing (CORS) for Actix applications

use std::collections::HashSet;

use http::{self, Method, HttpTryFrom, Uri};
use http::header::{self, HeaderName};

use error::{Result, ResponseError};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use middleware::{Middleware, Response, Started};
use httpcodes::HTTPBadRequest;

/// A set of errors that can occur during processing CORS
#[derive(Debug, Fail)]
pub enum Error {
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
    /// Credentials are allowed, but the Origin is set to "*". This is not allowed by W3C
    ///
    /// This is a misconfiguration. Check the docuemntation for `Cors`.
    #[fail(display="Credentials are allowed, but the Origin is set to \"*\"")]
    CredentialsWithWildcardOrigin,
}

impl ResponseError for Error {

    fn error_response(&self) -> HttpResponse {
        match *self {
            Error::BadOrigin => HTTPBadRequest.into(),
            _ => HTTPBadRequest.into()
        }
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
}

/// `Middleware` for Cross-origin resource sharing support
///
/// The Cors struct contains the settings for CORS requests to be validated and
/// for responses to be generated.
pub struct Cors {
    methods: HashSet<Method>,
    origins: AllOrSome<HashSet<Uri>>,
    headers: AllOrSome<HashSet<HeaderName>>,
    max_age: Option<usize>,
}

impl Cors {
    pub fn build() -> CorsBuilder {
        CorsBuilder {
            cors: Some(Cors {
                origins: AllOrSome::All,
                methods: HashSet::new(),
                headers: AllOrSome::All,
                max_age: None,
            }),
            methods: false,
            error: None,
        }
    }

    fn validate_origin<S>(&self, req: &mut HttpRequest<S>) -> Result<(), Error> {
        if let Some(hdr) = req.headers().get(header::ORIGIN) {
            if let Ok(origin) = hdr.to_str() {
                if let Ok(uri) = Uri::try_from(origin) {
                    return match self.origins {
                        AllOrSome::All => Ok(()),
                        AllOrSome::Some(ref allowed_origins) => {
                            allowed_origins
                                .get(&uri)
                                .and_then(|_| Some(()))
                                .ok_or_else(|| Error::OriginNotAllowed)
                        }
                    };
                }
            }
            Err(Error::BadOrigin)
        } else {
            Ok(())
        }
    }

    fn validate_allowed_method<S>(&self, req: &mut HttpRequest<S>) -> Result<(), Error> {
        if let Some(hdr) = req.headers().get(header::ACCESS_CONTROL_REQUEST_METHOD) {
            if let Ok(meth) = hdr.to_str() {
                if let Ok(method) = Method::try_from(meth) {
                    return self.methods.get(&method)
                        .and_then(|_| Some(()))
                        .ok_or_else(|| Error::MethodNotAllowed);
                }
            }
            Err(Error::BadRequestMethod)
        } else {
            Err(Error::MissingRequestMethod)
        }
    }
}

impl<S> Middleware<S> for Cors {

    fn start(&self, req: &mut HttpRequest<S>) -> Result<Started> {
        if Method::OPTIONS == *req.method() {
            self.validate_origin(req)?;
            self.validate_allowed_method(req)?;
        }
        Ok(Started::Done)
    }

    fn response(&self, _: &mut HttpRequest<S>, mut resp: HttpResponse) -> Result<Response> {
        Ok(Response::Done(resp))
    }
}

/// Structure that follows the builder pattern for building `Cors` middleware structs.
///
/// To construct a cors:
///
///   1. Call [`Cors::build`](struct.Cors.html#method.build) to start building.
///   2. Use any of the builder methods to set fields in the backend.
///   3. Call [finish](#method.finish) to retrieve the constructed backend.
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
            for m in methods.into_iter() {
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
            for h in headers.into_iter() {
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

    /// Finishes building and returns the built `Cors` instance.
    pub fn finish(&mut self) -> Result<Cors, http::Error> {
        if !self.methods {
            self.allowed_methods(vec![Method::GET, Method::HEAD,
                                    Method::POST, Method::OPTIONS, Method::PUT,
                                    Method::PATCH, Method::DELETE]);
        }

        if let Some(e) = self.error.take() {
            return Err(e)
        }

        Ok(self.cors.take().expect("cannot reuse CorsBuilder"))
    }
}

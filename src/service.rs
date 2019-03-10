use std::cell::{Ref, RefMut};
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_http::body::{Body, MessageBody, ResponseBody};
use actix_http::http::{HeaderMap, Method, Uri, Version};
use actix_http::{
    Error, Extensions, HttpMessage, Payload, PayloadStream, Request, RequestHead,
    Response, ResponseHead,
};
use actix_router::{Path, Resource, Url};
use futures::future::{ok, FutureResult, IntoFuture};

use crate::config::{AppConfig, ServiceConfig};
use crate::request::HttpRequest;
use crate::rmap::ResourceMap;

pub trait HttpServiceFactory<P> {
    fn register(self, config: &mut ServiceConfig<P>);
}

pub(crate) trait ServiceFactory<P> {
    fn register(&mut self, config: &mut ServiceConfig<P>);
}

pub(crate) struct ServiceFactoryWrapper<T, P> {
    factory: Option<T>,
    _t: PhantomData<P>,
}

impl<T, P> ServiceFactoryWrapper<T, P> {
    pub fn new(factory: T) -> Self {
        Self {
            factory: Some(factory),
            _t: PhantomData,
        }
    }
}

impl<T, P> ServiceFactory<P> for ServiceFactoryWrapper<T, P>
where
    T: HttpServiceFactory<P>,
{
    fn register(&mut self, config: &mut ServiceConfig<P>) {
        if let Some(item) = self.factory.take() {
            item.register(config)
        }
    }
}

pub struct ServiceRequest<P = PayloadStream> {
    req: HttpRequest,
    payload: Payload<P>,
}

impl<P> ServiceRequest<P> {
    pub(crate) fn new(
        path: Path<Url>,
        request: Request<P>,
        rmap: Rc<ResourceMap>,
        config: AppConfig,
    ) -> Self {
        let (head, payload) = request.into_parts();
        ServiceRequest {
            payload,
            req: HttpRequest::new(head, path, rmap, config),
        }
    }

    #[inline]
    pub fn into_request(self) -> HttpRequest {
        self.req
    }

    /// Create service response
    #[inline]
    pub fn into_response<B>(self, res: Response<B>) -> ServiceResponse<B> {
        ServiceResponse::new(self.req, res)
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<B, E: Into<Error>>(self, err: E) -> ServiceResponse<B> {
        let res: Response = err.into().into();
        ServiceResponse::new(self.req, res.into_body())
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        &self.req.head
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head_mut(&mut self) -> &mut RequestHead {
        &mut self.req.head
    }

    /// Request's uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.head().uri
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.head().method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    #[inline]
    /// Returns mutable Request's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head_mut().headers
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.head().uri.path()
    }

    /// The query string in the URL.
    ///
    /// E.g., id=10
    #[inline]
    pub fn query_string(&self) -> &str {
        if let Some(query) = self.uri().query().as_ref() {
            query
        } else {
            ""
        }
    }

    /// Get a reference to the Path parameters.
    ///
    /// Params is a container for url parameters.
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment.
    #[inline]
    pub fn match_info(&self) -> &Path<Url> {
        &self.req.path
    }

    #[inline]
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        &mut self.req.path
    }

    /// Service configuration
    #[inline]
    pub fn app_config(&self) -> &AppConfig {
        self.req.config()
    }

    /// Deconstruct request into parts
    pub fn into_parts(self) -> (HttpRequest, Payload<P>) {
        (self.req, self.payload)
    }
}

impl<P> Resource<Url> for ServiceRequest<P> {
    fn resource_path(&mut self) -> &mut Path<Url> {
        self.match_info_mut()
    }
}

impl<P> HttpMessage for ServiceRequest<P> {
    type Stream = P;

    #[inline]
    /// Returns Request's headers.
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Request extensions
    #[inline]
    fn extensions(&self) -> Ref<Extensions> {
        self.req.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.req.head.extensions_mut()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        std::mem::replace(&mut self.payload, Payload::None)
    }
}

impl<P> std::ops::Deref for ServiceRequest<P> {
    type Target = RequestHead;

    fn deref(&self) -> &RequestHead {
        self.req.head()
    }
}

impl<P> std::ops::DerefMut for ServiceRequest<P> {
    fn deref_mut(&mut self) -> &mut RequestHead {
        self.head_mut()
    }
}

impl<P> fmt::Debug for ServiceRequest<P> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "\nServiceRequest {:?} {}:{}",
            self.head().version,
            self.head().method,
            self.path()
        )?;
        if !self.query_string().is_empty() {
            writeln!(f, "  query: ?{:?}", self.query_string())?;
        }
        if !self.match_info().is_empty() {
            writeln!(f, "  params: {:?}", self.match_info())?;
        }
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

pub struct ServiceFromRequest<P> {
    req: HttpRequest,
    payload: Payload<P>,
    config: Option<Rc<Extensions>>,
}

impl<P> ServiceFromRequest<P> {
    pub(crate) fn new(req: ServiceRequest<P>, config: Option<Rc<Extensions>>) -> Self {
        Self {
            req: req.req,
            payload: req.payload,
            config,
        }
    }

    #[inline]
    pub fn into_request(self) -> HttpRequest {
        self.req
    }

    #[inline]
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        &mut self.req.path
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<E: Into<Error>>(self, err: E) -> ServiceResponse {
        ServiceResponse::new(self.req, err.into().into())
    }

    /// Load extractor configuration
    pub fn load_config<T: 'static>(&self) -> Option<&T> {
        if let Some(ref ext) = self.config {
            ext.get::<T>()
        } else {
            None
        }
    }
}

impl<P> std::ops::Deref for ServiceFromRequest<P> {
    type Target = HttpRequest;

    fn deref(&self) -> &HttpRequest {
        &self.req
    }
}

impl<P> HttpMessage for ServiceFromRequest<P> {
    type Stream = P;

    #[inline]
    fn headers(&self) -> &HeaderMap {
        self.req.headers()
    }

    /// Request extensions
    #[inline]
    fn extensions(&self) -> Ref<Extensions> {
        self.req.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.req.head.extensions_mut()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        std::mem::replace(&mut self.payload, Payload::None)
    }
}

pub struct ServiceResponse<B = Body> {
    request: HttpRequest,
    response: Response<B>,
}

impl<B> ServiceResponse<B> {
    /// Create service response instance
    pub fn new(request: HttpRequest, response: Response<B>) -> Self {
        ServiceResponse { request, response }
    }

    /// Create service response from the error
    pub fn from_err<E: Into<Error>>(err: E, request: HttpRequest) -> Self {
        let e: Error = err.into();
        let res: Response = e.into();
        ServiceResponse {
            request,
            response: res.into_body(),
        }
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<E: Into<Error>>(self, err: E) -> Self {
        Self::from_err(err, self.request)
    }

    /// Create service response
    #[inline]
    pub fn into_response<B1>(self, response: Response<B1>) -> ServiceResponse<B1> {
        ServiceResponse::new(self.request, response)
    }

    /// Get reference to original request
    #[inline]
    pub fn request(&self) -> &HttpRequest {
        &self.request
    }

    /// Get reference to response
    #[inline]
    pub fn response(&self) -> &Response<B> {
        &self.response
    }

    /// Get mutable reference to response
    #[inline]
    pub fn response_mut(&mut self) -> &mut Response<B> {
        &mut self.response
    }

    /// Execute closure and in case of error convert it to response.
    pub fn checked_expr<F, E>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut Self) -> Result<(), E>,
        E: Into<Error>,
    {
        match f(&mut self) {
            Ok(_) => self,
            Err(err) => {
                let res: Response = err.into().into();
                ServiceResponse::new(self.request, res.into_body())
            }
        }
    }

    /// Extract response body
    pub fn take_body(&mut self) -> ResponseBody<B> {
        self.response.take_body()
    }
}

impl<B> ServiceResponse<B> {
    /// Set a new body
    pub fn map_body<F, B2>(self, f: F) -> ServiceResponse<B2>
    where
        F: FnOnce(&mut ResponseHead, ResponseBody<B>) -> ResponseBody<B2>,
    {
        let response = self.response.map_body(f);

        ServiceResponse {
            response,
            request: self.request,
        }
    }
}

impl<B> std::ops::Deref for ServiceResponse<B> {
    type Target = Response<B>;

    fn deref(&self) -> &Response<B> {
        self.response()
    }
}

impl<B> std::ops::DerefMut for ServiceResponse<B> {
    fn deref_mut(&mut self) -> &mut Response<B> {
        self.response_mut()
    }
}

impl<B> Into<Response<B>> for ServiceResponse<B> {
    fn into(self) -> Response<B> {
        self.response
    }
}

impl<B> IntoFuture for ServiceResponse<B> {
    type Item = ServiceResponse<B>;
    type Error = Error;
    type Future = FutureResult<ServiceResponse<B>, Error>;

    fn into_future(self) -> Self::Future {
        ok(self)
    }
}

impl<B: MessageBody> fmt::Debug for ServiceResponse<B> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = writeln!(
            f,
            "\nServiceResponse {:?} {}{}",
            self.response.head().version,
            self.response.head().status,
            self.response.head().reason.unwrap_or(""),
        );
        let _ = writeln!(f, "  headers:");
        for (key, val) in self.response.head().headers.iter() {
            let _ = writeln!(f, "    {:?}: {:?}", key, val);
        }
        let _ = writeln!(f, "  body: {:?}", self.response.body().length());
        res
    }
}

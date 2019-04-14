use std::cell::{Ref, RefMut};
use std::fmt;

use actix_http::body::{Body, MessageBody, ResponseBody};
use actix_http::http::{HeaderMap, Method, StatusCode, Uri, Version};
use actix_http::{
    Error, Extensions, HttpMessage, Payload, PayloadStream, RequestHead, Response,
    ResponseHead,
};
use actix_router::{Path, Resource, Url};
use futures::future::{ok, FutureResult, IntoFuture};

use crate::config::{AppConfig, ServiceConfig};
use crate::data::Data;
use crate::request::HttpRequest;

pub trait HttpServiceFactory {
    fn register(self, config: &mut ServiceConfig);
}

pub(crate) trait ServiceFactory {
    fn register(&mut self, config: &mut ServiceConfig);
}

pub(crate) struct ServiceFactoryWrapper<T> {
    factory: Option<T>,
}

impl<T> ServiceFactoryWrapper<T> {
    pub fn new(factory: T) -> Self {
        Self {
            factory: Some(factory),
        }
    }
}

impl<T> ServiceFactory for ServiceFactoryWrapper<T>
where
    T: HttpServiceFactory,
{
    fn register(&mut self, config: &mut ServiceConfig) {
        if let Some(item) = self.factory.take() {
            item.register(config)
        }
    }
}

pub struct ServiceRequest {
    req: HttpRequest,
    payload: Payload,
}

impl ServiceRequest {
    /// Construct service request from parts
    pub fn from_parts(req: HttpRequest, payload: Payload) -> Self {
        ServiceRequest { req, payload }
    }

    /// Deconstruct request into parts
    pub fn into_parts(self) -> (HttpRequest, Payload) {
        (self.req, self.payload)
    }

    /// Create service response
    #[inline]
    pub fn into_response<B, R: Into<Response<B>>>(self, res: R) -> ServiceResponse<B> {
        ServiceResponse::new(self.req, res.into())
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
        &self.req.head()
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head_mut(&mut self) -> &mut RequestHead {
        self.req.head_mut()
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
    /// Returns request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    #[inline]
    /// Returns mutable request's headers.
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
        self.req.match_info()
    }

    #[inline]
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        self.req.match_info_mut()
    }

    /// Service configuration
    #[inline]
    pub fn app_config(&self) -> &AppConfig {
        self.req.app_config()
    }

    /// Get an application data stored with `App::data()` method during
    /// application configuration.
    pub fn app_data<T: 'static>(&self) -> Option<Data<T>> {
        if let Some(st) = self.req.app_config().extensions().get::<Data<T>>() {
            Some(st.clone())
        } else {
            None
        }
    }
}

impl Resource<Url> for ServiceRequest {
    fn resource_path(&mut self) -> &mut Path<Url> {
        self.match_info_mut()
    }
}

impl HttpMessage for ServiceRequest {
    type Stream = PayloadStream;

    #[inline]
    /// Returns Request's headers.
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Request extensions
    #[inline]
    fn extensions(&self) -> Ref<Extensions> {
        self.req.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.req.extensions_mut()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        std::mem::replace(&mut self.payload, Payload::None)
    }
}

impl fmt::Debug for ServiceRequest {
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

    /// Get the response status code
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.response.status()
    }

    #[inline]
    /// Returns response's headers.
    pub fn headers(&self) -> &HeaderMap {
        self.response.headers()
    }

    #[inline]
    /// Returns mutable response's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.response.headers_mut()
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
        let _ = writeln!(f, "  body: {:?}", self.response.body().size());
        res
    }
}

#[cfg(test)]
mod tests {
    use crate::test::TestRequest;
    use crate::HttpResponse;

    #[test]
    fn test_fmt_debug() {
        let req = TestRequest::get()
            .uri("/index.html?test=1")
            .header("x-test", "111")
            .to_srv_request();
        let s = format!("{:?}", req);
        assert!(s.contains("ServiceRequest"));
        assert!(s.contains("test=1"));
        assert!(s.contains("x-test"));

        let res = HttpResponse::Ok().header("x-test", "111").finish();
        let res = TestRequest::post()
            .uri("/index.html?test=1")
            .to_srv_response(res);

        let s = format!("{:?}", res);
        assert!(s.contains("ServiceResponse"));
        assert!(s.contains("x-test"));
    }
}

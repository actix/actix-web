use std::cell::{Ref, RefMut};
use std::rc::Rc;
use std::{fmt, net};

use actix_http::body::{Body, MessageBody, ResponseBody};
use actix_http::http::{HeaderMap, Method, StatusCode, Uri, Version};
use actix_http::{
    Error, Extensions, HttpMessage, Payload, PayloadStream, RequestHead, Response,
    ResponseHead,
};
use actix_router::{Path, Resource, ResourceDef, Url};
use actix_service::{IntoNewService, NewService};
use futures::future::{ok, FutureResult, IntoFuture};

use crate::config::{AppConfig, AppService};
use crate::data::Data;
use crate::dev::insert_slash;
use crate::guard::Guard;
use crate::info::ConnectionInfo;
use crate::request::HttpRequest;
use crate::rmap::ResourceMap;

pub trait HttpServiceFactory {
    fn register(self, config: &mut AppService);
}

pub(crate) trait ServiceFactory {
    fn register(&mut self, config: &mut AppService);
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
    fn register(&mut self, config: &mut AppService) {
        if let Some(item) = self.factory.take() {
            item.register(config)
        }
    }
}

/// An service http request
///
/// ServiceRequest allows mutable access to request's internal structures
pub struct ServiceRequest(HttpRequest);

impl ServiceRequest {
    /// Construct service request
    pub(crate) fn new(req: HttpRequest) -> Self {
        ServiceRequest(req)
    }

    /// Deconstruct request into parts
    pub fn into_parts(mut self) -> (HttpRequest, Payload) {
        let pl = Rc::get_mut(&mut (self.0).0).unwrap().payload.take();
        (self.0, pl)
    }

    /// Construct request from parts.
    ///
    /// `ServiceRequest` can be re-constructed only if `req` hasnt been cloned.
    pub fn from_parts(
        mut req: HttpRequest,
        pl: Payload,
    ) -> Result<Self, (HttpRequest, Payload)> {
        if Rc::strong_count(&req.0) == 1 && Rc::weak_count(&req.0) == 0 {
            Rc::get_mut(&mut req.0).unwrap().payload = pl;
            Ok(ServiceRequest(req))
        } else {
            Err((req, pl))
        }
    }

    /// Construct request from request.
    ///
    /// `HttpRequest` implements `Clone` trait via `Rc` type. `ServiceRequest`
    /// can be re-constructed only if rc's strong pointers count eq 1 and
    /// weak pointers count is 0.
    pub fn from_request(req: HttpRequest) -> Result<Self, HttpRequest> {
        if Rc::strong_count(&req.0) == 1 && Rc::weak_count(&req.0) == 0 {
            Ok(ServiceRequest(req))
        } else {
            Err(req)
        }
    }

    /// Create service response
    #[inline]
    pub fn into_response<B, R: Into<Response<B>>>(self, res: R) -> ServiceResponse<B> {
        ServiceResponse::new(self.0, res.into())
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<B, E: Into<Error>>(self, err: E) -> ServiceResponse<B> {
        let res: Response = err.into().into();
        ServiceResponse::new(self.0, res.into_body())
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head(&self) -> &RequestHead {
        &self.0.head()
    }

    /// This method returns reference to the request head
    #[inline]
    pub fn head_mut(&mut self) -> &mut RequestHead {
        self.0.head_mut()
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

    /// Peer socket address
    ///
    /// Peer address is actual socket address, if proxy is used in front of
    /// actix http server, then peer address would be address of this proxy.
    ///
    /// To get client connection information `ConnectionInfo` should be used.
    #[inline]
    pub fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.head().peer_addr
    }

    /// Get *ConnectionInfo* for the current request.
    #[inline]
    pub fn connection_info(&self) -> Ref<ConnectionInfo> {
        ConnectionInfo::get(self.head(), &*self.app_config())
    }

    /// Get a reference to the Path parameters.
    ///
    /// Params is a container for url parameters.
    /// A variable segment is specified in the form `{identifier}`,
    /// where the identifier can be used later in a request handler to
    /// access the matched value for that segment.
    #[inline]
    pub fn match_info(&self) -> &Path<Url> {
        self.0.match_info()
    }

    #[inline]
    /// Get a mutable reference to the Path parameters.
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        self.0.match_info_mut()
    }

    #[inline]
    /// Get a reference to a `ResourceMap` of current application.
    pub fn resource_map(&self) -> &ResourceMap {
        self.0.resource_map()
    }

    /// Service configuration
    #[inline]
    pub fn app_config(&self) -> &AppConfig {
        self.0.app_config()
    }

    /// Get an application data stored with `App::data()` method during
    /// application configuration.
    pub fn app_data<T: 'static>(&self) -> Option<Data<T>> {
        if let Some(st) = (self.0).0.app_data.get::<Data<T>>() {
            Some(st.clone())
        } else {
            None
        }
    }

    /// Set request payload.
    pub fn set_payload(&mut self, payload: Payload) {
        Rc::get_mut(&mut (self.0).0).unwrap().payload = payload;
    }

    #[doc(hidden)]
    /// Set new app data container
    pub fn set_data_container(&mut self, extensions: Rc<Extensions>) {
        Rc::get_mut(&mut (self.0).0).unwrap().app_data = extensions;
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
        self.0.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.0.extensions_mut()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        Rc::get_mut(&mut (self.0).0).unwrap().payload.take()
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

pub struct WebService {
    rdef: String,
    name: Option<String>,
    guards: Vec<Box<dyn Guard>>,
}

impl WebService {
    /// Create new `WebService` instance.
    pub fn new(path: &str) -> Self {
        WebService {
            rdef: path.to_string(),
            name: None,
            guards: Vec::new(),
        }
    }

    /// Set service name.
    ///
    /// Name is used for url generation.
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    /// Add match guard to a web service.
    ///
    /// ```rust
    /// use actix_web::{web, guard, dev, App, HttpResponse};
    ///
    /// fn index(req: dev::ServiceRequest) -> dev::ServiceResponse {
    ///     req.into_response(HttpResponse::Ok().finish())
    /// }
    ///
    /// fn main() {
    ///     let app = App::new()
    ///         .service(
    ///             web::service("/app")
    ///                 .guard(guard::Header("content-type", "text/plain"))
    ///                 .finish(index)
    ///         );
    /// }
    /// ```
    pub fn guard<G: Guard + 'static>(mut self, guard: G) -> Self {
        self.guards.push(Box::new(guard));
        self
    }

    /// Set a service factory implementation and generate web service.
    pub fn finish<T, F>(self, service: F) -> impl HttpServiceFactory
    where
        F: IntoNewService<T>,
        T: NewService<
                Config = (),
                Request = ServiceRequest,
                Response = ServiceResponse,
                Error = Error,
                InitError = (),
            > + 'static,
    {
        WebServiceImpl {
            srv: service.into_new_service(),
            rdef: self.rdef,
            name: self.name,
            guards: self.guards,
        }
    }
}

struct WebServiceImpl<T> {
    srv: T,
    rdef: String,
    name: Option<String>,
    guards: Vec<Box<dyn Guard>>,
}

impl<T> HttpServiceFactory for WebServiceImpl<T>
where
    T: NewService<
            Config = (),
            Request = ServiceRequest,
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        > + 'static,
{
    fn register(mut self, config: &mut AppService) {
        let guards = if self.guards.is_empty() {
            None
        } else {
            Some(std::mem::replace(&mut self.guards, Vec::new()))
        };

        let mut rdef = if config.is_root() || !self.rdef.is_empty() {
            ResourceDef::new(&insert_slash(&self.rdef))
        } else {
            ResourceDef::new(&self.rdef)
        };
        if let Some(ref name) = self.name {
            *rdef.name_mut() = name.clone();
        }
        config.register_service(rdef, guards, self.srv, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::{call_service, init_service, TestRequest};
    use crate::{guard, http, web, App, HttpResponse};

    #[test]
    fn test_service_request() {
        let req = TestRequest::default().to_srv_request();
        let (r, pl) = req.into_parts();
        assert!(ServiceRequest::from_parts(r, pl).is_ok());

        let req = TestRequest::default().to_srv_request();
        let (r, pl) = req.into_parts();
        let _r2 = r.clone();
        assert!(ServiceRequest::from_parts(r, pl).is_err());

        let req = TestRequest::default().to_srv_request();
        let (r, _pl) = req.into_parts();
        assert!(ServiceRequest::from_request(r).is_ok());

        let req = TestRequest::default().to_srv_request();
        let (r, _pl) = req.into_parts();
        let _r2 = r.clone();
        assert!(ServiceRequest::from_request(r).is_err());
    }

    #[test]
    fn test_service() {
        let mut srv = init_service(
            App::new().service(web::service("/test").name("test").finish(
                |req: ServiceRequest| req.into_response(HttpResponse::Ok().finish()),
            )),
        );
        let req = TestRequest::with_uri("/test").to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), http::StatusCode::OK);

        let mut srv = init_service(
            App::new().service(web::service("/test").guard(guard::Get()).finish(
                |req: ServiceRequest| req.into_response(HttpResponse::Ok().finish()),
            )),
        );
        let req = TestRequest::with_uri("/test")
            .method(http::Method::PUT)
            .to_request();
        let resp = call_service(&mut srv, req);
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

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

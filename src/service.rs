use std::cell::{Ref, RefMut};
use std::rc::Rc;
use std::{fmt, net};

use actix_http::body::{Body, MessageBody, ResponseBody};
use actix_http::http::{HeaderMap, Method, StatusCode, Uri, Version};
use actix_http::{
    Error, Extensions, HttpMessage, Payload, PayloadStream, RequestHead, Response, ResponseHead,
};
use actix_router::{IntoPattern, Path, Resource, ResourceDef, Url};
use actix_service::{IntoServiceFactory, ServiceFactory};

use crate::dev::insert_slash;
use crate::guard::Guard;
use crate::info::ConnectionInfo;
use crate::request::HttpRequest;
use crate::rmap::ResourceMap;
use crate::{
    config::{AppConfig, AppService},
    HttpResponse,
};

pub trait HttpServiceFactory {
    fn register(self, config: &mut AppService);
}

impl<T: HttpServiceFactory> HttpServiceFactory for Vec<T> {
    fn register(self, config: &mut AppService) {
        self.into_iter()
            .for_each(|factory| factory.register(config));
    }
}

pub(crate) trait AppServiceFactory {
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

impl<T> AppServiceFactory for ServiceFactoryWrapper<T>
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
pub struct ServiceRequest {
    req: HttpRequest,
    payload: Payload,
}

impl ServiceRequest {
    /// Construct service request
    pub(crate) fn new(req: HttpRequest, payload: Payload) -> Self {
        Self { req, payload }
    }

    /// Construct service request.
    #[doc(hidden)]
    pub fn __priv_test_new(req: HttpRequest, payload: Payload) -> Self {
        Self::new(req, payload)
    }

    /// Deconstruct request into parts
    #[inline]
    pub fn into_parts(self) -> (HttpRequest, Payload) {
        (self.req, self.payload)
    }

    /// Construct request from parts.
    pub fn from_parts(req: HttpRequest, payload: Payload) -> Self {
        Self { req, payload }
    }

    /// Construct request from request.
    ///
    /// The returned `ServiceRequest` would have no payload.
    pub fn from_request(req: HttpRequest) -> Self {
        ServiceRequest {
            req,
            payload: Payload::None,
        }
    }

    /// Create service response
    #[inline]
    pub fn into_response<B, R: Into<Response<B>>>(self, res: R) -> ServiceResponse<B> {
        let res = HttpResponse::from(res.into());
        ServiceResponse::new(self.req, res)
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<B, E: Into<Error>>(self, err: E) -> ServiceResponse<B> {
        let res = HttpResponse::from_error(err.into());
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

    /// Peer socket address.
    ///
    /// Peer address is the directly connected peer's socket address. If a proxy is used in front of
    /// the Actix Web server, then it would be address of this proxy.
    ///
    /// To get client connection information `ConnectionInfo` should be used.
    ///
    /// Will only return None when called in unit tests.
    #[inline]
    pub fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.head().peer_addr
    }

    /// Get *ConnectionInfo* for the current request.
    #[inline]
    pub fn connection_info(&self) -> Ref<'_, ConnectionInfo> {
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
        self.req.match_info()
    }

    /// Counterpart to [`HttpRequest::match_name`](super::HttpRequest::match_name()).
    #[inline]
    pub fn match_name(&self) -> Option<&str> {
        self.req.match_name()
    }

    /// Counterpart to [`HttpRequest::match_pattern`](super::HttpRequest::match_pattern()).
    #[inline]
    pub fn match_pattern(&self) -> Option<String> {
        self.req.match_pattern()
    }

    #[inline]
    /// Get a mutable reference to the Path parameters.
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        self.req.match_info_mut()
    }

    #[inline]
    /// Get a reference to a `ResourceMap` of current application.
    pub fn resource_map(&self) -> &ResourceMap {
        self.req.resource_map()
    }

    /// Service configuration
    #[inline]
    pub fn app_config(&self) -> &AppConfig {
        self.req.app_config()
    }

    /// Counterpart to [`HttpRequest::app_data`](super::HttpRequest::app_data()).
    pub fn app_data<T: 'static>(&self) -> Option<&T> {
        for container in self.req.inner.app_data.iter().rev() {
            if let Some(data) = container.get::<T>() {
                return Some(data);
            }
        }

        None
    }

    /// Set request payload.
    pub fn set_payload(&mut self, payload: Payload) {
        self.payload = payload;
    }

    /// Add data container to request's resolution set.
    ///
    /// In middleware, prefer [`extensions_mut`](ServiceRequest::extensions_mut) for request-local
    /// data since it is assumed that the same app data is presented for every request.
    pub fn add_data_container(&mut self, extensions: Rc<Extensions>) {
        Rc::get_mut(&mut (self.req).inner)
            .unwrap()
            .app_data
            .push(extensions);
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
    fn extensions(&self) -> Ref<'_, Extensions> {
        self.req.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.req.extensions_mut()
    }

    #[inline]
    fn take_payload(&mut self) -> Payload<Self::Stream> {
        self.payload.take()
    }
}

impl fmt::Debug for ServiceRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    response: HttpResponse<B>,
}

impl<B> ServiceResponse<B> {
    /// Create service response instance
    pub fn new(request: HttpRequest, response: HttpResponse<B>) -> Self {
        ServiceResponse { request, response }
    }

    /// Create service response from the error
    pub fn from_err<E: Into<Error>>(err: E, request: HttpRequest) -> Self {
        let response = HttpResponse::from_error(err.into()).into_body();
        ServiceResponse { request, response }
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<E: Into<Error>>(self, err: E) -> Self {
        Self::from_err(err, self.request)
    }

    /// Create service response
    #[inline]
    pub fn into_response<B1>(self, response: HttpResponse<B1>) -> ServiceResponse<B1> {
        ServiceResponse::new(self.request, response)
    }

    /// Get reference to original request
    #[inline]
    pub fn request(&self) -> &HttpRequest {
        &self.request
    }

    /// Get reference to response
    #[inline]
    pub fn response(&self) -> &HttpResponse<B> {
        &self.response
    }

    /// Get mutable reference to response
    #[inline]
    pub fn response_mut(&mut self) -> &mut HttpResponse<B> {
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

    /// Returns mutable response's headers.
    #[inline]
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
                let res = HttpResponse::from_error(err.into());
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

impl<B> From<ServiceResponse<B>> for HttpResponse<B> {
    fn from(res: ServiceResponse<B>) -> HttpResponse<B> {
        res.response
    }
}

impl<B> From<ServiceResponse<B>> for Response<B> {
    fn from(res: ServiceResponse<B>) -> Response<B> {
        res.response.into()
    }
}

impl<B: MessageBody> fmt::Debug for ServiceResponse<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    rdef: Vec<String>,
    name: Option<String>,
    guards: Vec<Box<dyn Guard>>,
}

impl WebService {
    /// Create new `WebService` instance.
    pub fn new<T: IntoPattern>(path: T) -> Self {
        WebService {
            rdef: path.patterns(),
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
    /// ```
    /// use actix_web::{web, guard, dev, App, Error, HttpResponse};
    ///
    /// async fn index(req: dev::ServiceRequest) -> Result<dev::ServiceResponse, Error> {
    ///     Ok(req.into_response(HttpResponse::Ok().finish()))
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
        F: IntoServiceFactory<T, ServiceRequest>,
        T: ServiceFactory<
                ServiceRequest,
                Config = (),
                Response = ServiceResponse,
                Error = Error,
                InitError = (),
            > + 'static,
    {
        WebServiceImpl {
            srv: service.into_factory(),
            rdef: self.rdef,
            name: self.name,
            guards: self.guards,
        }
    }
}

struct WebServiceImpl<T> {
    srv: T,
    rdef: Vec<String>,
    name: Option<String>,
    guards: Vec<Box<dyn Guard>>,
}

impl<T> HttpServiceFactory for WebServiceImpl<T>
where
    T: ServiceFactory<
            ServiceRequest,
            Config = (),
            Response = ServiceResponse,
            Error = Error,
            InitError = (),
        > + 'static,
{
    fn register(mut self, config: &mut AppService) {
        let guards = if self.guards.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.guards))
        };

        let mut rdef = if config.is_root() || !self.rdef.is_empty() {
            ResourceDef::new(insert_slash(self.rdef))
        } else {
            ResourceDef::new(self.rdef)
        };
        if let Some(ref name) = self.name {
            *rdef.name_mut() = name.clone();
        }
        config.register_service(rdef, guards, self.srv, None)
    }
}

/// Macro helping register different types of services at the sametime.
///
/// The service type must be implementing [`HttpServiceFactory`](self::HttpServiceFactory) trait.
///
/// The max number of services can be grouped together is 12.
///
/// # Examples
///
/// ```
/// use actix_web::{services, web, App};
///
/// let services = services![
///     web::resource("/test2").to(|| async { "test2" }),
///     web::scope("/test3").route("/", web::get().to(|| async { "test3" }))
/// ];
///
/// let app = App::new().service(services);
///
/// // services macro just convert multiple services to a tuple.
/// // below would also work without importing the macro.
/// let app = App::new().service((
///     web::resource("/test2").to(|| async { "test2" }),
///     web::scope("/test3").route("/", web::get().to(|| async { "test3" }))
/// ));
/// ```
#[macro_export]
macro_rules! services {
    ($($x:expr),+ $(,)?) => {
        ($($x,)+)
    }
}

/// HttpServiceFactory trait impl for tuples
macro_rules! service_tuple ({ $($T:ident)+ } => {
    impl<$($T: HttpServiceFactory),+> HttpServiceFactory for ($($T,)+) {
        #[allow(non_snake_case)]
        fn register(self, config: &mut AppService) {
            let ($($T,)*) = self;
            $($T.register(config);)+
        }
    }
});

service_tuple! { A }
service_tuple! { A B }
service_tuple! { A B C }
service_tuple! { A B C D }
service_tuple! { A B C D E }
service_tuple! { A B C D E F }
service_tuple! { A B C D E F G }
service_tuple! { A B C D E F G H }
service_tuple! { A B C D E F G H I }
service_tuple! { A B C D E F G H I J }
service_tuple! { A B C D E F G H I J K }
service_tuple! { A B C D E F G H I J K L }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test::{init_service, TestRequest};
    use crate::{guard, http, web, App, HttpResponse};
    use actix_service::Service;
    use actix_utils::future::ok;

    #[actix_rt::test]
    async fn test_service() {
        let srv = init_service(
            App::new().service(web::service("/test").name("test").finish(
                |req: ServiceRequest| ok(req.into_response(HttpResponse::Ok().finish())),
            )),
        )
        .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let srv = init_service(
            App::new().service(web::service("/test").guard(guard::Get()).finish(
                |req: ServiceRequest| ok(req.into_response(HttpResponse::Ok().finish())),
            )),
        )
        .await;
        let req = TestRequest::with_uri("/test")
            .method(http::Method::PUT)
            .to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_service_data() {
        let srv =
            init_service(
                App::new()
                    .data(42u32)
                    .service(web::service("/test").name("test").finish(
                        |req: ServiceRequest| {
                            assert_eq!(req.app_data::<web::Data<u32>>().unwrap().as_ref(), &42);
                            ok(req.into_response(HttpResponse::Ok().finish()))
                        },
                    )),
            )
            .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
    }

    #[test]
    fn test_fmt_debug() {
        let req = TestRequest::get()
            .uri("/index.html?test=1")
            .insert_header(("x-test", "111"))
            .to_srv_request();
        let s = format!("{:?}", req);
        assert!(s.contains("ServiceRequest"));
        assert!(s.contains("test=1"));
        assert!(s.contains("x-test"));

        let res = HttpResponse::Ok().insert_header(("x-test", "111")).finish();
        let res = TestRequest::post()
            .uri("/index.html?test=1")
            .to_srv_response(res);

        let s = format!("{:?}", res);
        assert!(s.contains("ServiceResponse"));
        assert!(s.contains("x-test"));
    }

    #[actix_rt::test]
    async fn test_services_macro() {
        let scoped = services![
            web::service("/scoped_test1").name("scoped_test1").finish(
                |req: ServiceRequest| async {
                    Ok(req.into_response(HttpResponse::Ok().finish()))
                }
            ),
            web::resource("/scoped_test2").to(|| async { "test2" }),
        ];

        let services = services![
            web::service("/test1")
                .name("test")
                .finish(|req: ServiceRequest| async {
                    Ok(req.into_response(HttpResponse::Ok().finish()))
                }),
            web::resource("/test2").to(|| async { "test2" }),
            web::scope("/test3").service(scoped)
        ];

        let srv = init_service(App::new().service(services)).await;

        let req = TestRequest::with_uri("/test1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let req = TestRequest::with_uri("/test2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let req = TestRequest::with_uri("/test3/scoped_test1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let req = TestRequest::with_uri("/test3/scoped_test2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_services_vec() {
        let services = vec![
            web::resource("/test1").to(|| async { "test1" }),
            web::resource("/test2").to(|| async { "test2" }),
        ];

        let scoped = vec![
            web::resource("/scoped_test1").to(|| async { "test1" }),
            web::resource("/scoped_test2").to(|| async { "test2" }),
        ];

        let srv = init_service(
            App::new()
                .service(services)
                .service(web::scope("/test3").service(scoped)),
        )
        .await;

        let req = TestRequest::with_uri("/test1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let req = TestRequest::with_uri("/test2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let req = TestRequest::with_uri("/test3/scoped_test1").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let req = TestRequest::with_uri("/test3/scoped_test2").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
    }
}

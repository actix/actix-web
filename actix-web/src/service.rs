use std::{
    cell::{Ref, RefMut},
    fmt, net,
    rc::Rc,
};

use actix_http::{
    body::{BoxBody, EitherBody, MessageBody},
    header::HeaderMap,
    BoxedPayloadStream, Extensions, HttpMessage, Method, Payload, RequestHead, Response,
    ResponseHead, StatusCode, Uri, Version,
};
use actix_router::{IntoPatterns, Path, Patterns, Resource, ResourceDef, Url};
use actix_service::{
    boxed::{BoxService, BoxServiceFactory},
    IntoServiceFactory, ServiceFactory,
};
#[cfg(feature = "cookies")]
use cookie::{Cookie, ParseError as CookieParseError};

use crate::{
    config::{AppConfig, AppService},
    dev::ensure_leading_slash,
    guard::{Guard, GuardContext},
    info::ConnectionInfo,
    rmap::ResourceMap,
    Error, FromRequest, HttpRequest, HttpResponse,
};

pub(crate) type BoxedHttpService = BoxService<ServiceRequest, ServiceResponse<BoxBody>, Error>;
pub(crate) type BoxedHttpServiceFactory =
    BoxServiceFactory<(), ServiceRequest, ServiceResponse<BoxBody>, Error, ()>;

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

/// A service level request wrapper.
///
/// Allows mutable access to request's internal structures.
pub struct ServiceRequest {
    req: HttpRequest,
    payload: Payload,
}

impl ServiceRequest {
    /// Construct `ServiceRequest` from parts.
    pub(crate) fn new(req: HttpRequest, payload: Payload) -> Self {
        Self { req, payload }
    }

    /// Deconstruct `ServiceRequest` into inner parts.
    #[inline]
    pub fn into_parts(self) -> (HttpRequest, Payload) {
        (self.req, self.payload)
    }

    /// Returns mutable accessors to inner parts.
    #[inline]
    pub fn parts_mut(&mut self) -> (&mut HttpRequest, &mut Payload) {
        (&mut self.req, &mut self.payload)
    }

    /// Returns immutable accessors to inner parts.
    #[inline]
    pub fn parts(&self) -> (&HttpRequest, &Payload) {
        (&self.req, &self.payload)
    }

    /// Returns immutable accessor to inner [`HttpRequest`].
    #[inline]
    pub fn request(&self) -> &HttpRequest {
        &self.req
    }

    /// Derives a type from this request using an [extractor](crate::FromRequest).
    ///
    /// Returns the `T` extractor's `Future` type which can be `await`ed. This is particularly handy
    /// when you want to use an extractor in a middleware implementation.
    ///
    /// # Examples
    /// ```
    /// use actix_web::{
    ///     dev::{ServiceRequest, ServiceResponse},
    ///     web::Path, Error
    /// };
    ///
    /// async fn my_helper(mut srv_req: ServiceRequest) -> Result<ServiceResponse, Error> {
    ///     let path = srv_req.extract::<Path<(String, u32)>>().await?;
    ///     // [...]
    /// #   todo!()
    /// }
    /// ```
    pub fn extract<T>(&mut self) -> <T as FromRequest>::Future
    where
        T: FromRequest,
    {
        T::from_request(&self.req, &mut self.payload)
    }

    /// Construct request from parts.
    pub fn from_parts(req: HttpRequest, payload: Payload) -> Self {
        #[cfg(debug_assertions)]
        if Rc::strong_count(&req.inner) > 1 {
            log::warn!("Cloning an `HttpRequest` might cause panics.");
        }

        Self { req, payload }
    }

    /// Construct `ServiceRequest` with no payload from given `HttpRequest`.
    #[inline]
    pub fn from_request(req: HttpRequest) -> Self {
        ServiceRequest {
            req,
            payload: Payload::None,
        }
    }

    /// Create `ServiceResponse` from this request and given response.
    #[inline]
    pub fn into_response<B, R: Into<Response<B>>>(self, res: R) -> ServiceResponse<B> {
        let res = HttpResponse::from(res.into());
        ServiceResponse::new(self.req, res)
    }

    /// Create `ServiceResponse` from this request and given error.
    #[inline]
    pub fn error_response<E: Into<Error>>(self, err: E) -> ServiceResponse {
        let res = HttpResponse::from_error(err.into());
        ServiceResponse::new(self.req, res)
    }

    /// Returns a reference to the request head.
    #[inline]
    pub fn head(&self) -> &RequestHead {
        self.req.head()
    }

    /// Returns a mutable reference to the request head.
    #[inline]
    pub fn head_mut(&mut self) -> &mut RequestHead {
        self.req.head_mut()
    }

    /// Returns the request URI.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.head().uri
    }

    /// Returns the request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.head().method
    }

    /// Returns the request version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    /// Returns a reference to request headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Returns a mutable reference to request headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head_mut().headers
    }

    /// Returns request path.
    #[inline]
    pub fn path(&self) -> &str {
        self.head().uri.path()
    }

    /// Counterpart to [`HttpRequest::query_string`].
    #[inline]
    pub fn query_string(&self) -> &str {
        self.req.query_string()
    }

    /// Returns peer's socket address.
    ///
    /// See [`HttpRequest::peer_addr`] for more details.
    ///
    /// [`HttpRequest::peer_addr`]: crate::HttpRequest::peer_addr
    #[inline]
    pub fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.head().peer_addr
    }

    /// Returns a reference to connection info.
    #[inline]
    pub fn connection_info(&self) -> Ref<'_, ConnectionInfo> {
        self.req.connection_info()
    }

    /// Counterpart to [`HttpRequest::match_info`].
    #[inline]
    pub fn match_info(&self) -> &Path<Url> {
        self.req.match_info()
    }

    /// Returns a mutable reference to the path match information.
    #[inline]
    pub fn match_info_mut(&mut self) -> &mut Path<Url> {
        self.req.match_info_mut()
    }

    /// Counterpart to [`HttpRequest::match_name`].
    #[inline]
    pub fn match_name(&self) -> Option<&str> {
        self.req.match_name()
    }

    /// Counterpart to [`HttpRequest::match_pattern`].
    #[inline]
    pub fn match_pattern(&self) -> Option<String> {
        self.req.match_pattern()
    }

    /// Returns a reference to the application's resource map.
    /// Counterpart to [`HttpRequest::resource_map`].
    #[inline]
    pub fn resource_map(&self) -> &ResourceMap {
        self.req.resource_map()
    }

    /// Counterpart to [`HttpRequest::app_config`].
    #[inline]
    pub fn app_config(&self) -> &AppConfig {
        self.req.app_config()
    }

    /// Counterpart to [`HttpRequest::app_data`].
    #[inline]
    pub fn app_data<T: 'static>(&self) -> Option<&T> {
        for container in self.req.inner.app_data.iter().rev() {
            if let Some(data) = container.get::<T>() {
                return Some(data);
            }
        }

        None
    }

    /// Counterpart to [`HttpRequest::conn_data`].
    #[inline]
    pub fn conn_data<T: 'static>(&self) -> Option<&T> {
        self.req.conn_data()
    }

    /// Return request cookies.
    #[cfg(feature = "cookies")]
    #[inline]
    pub fn cookies(&self) -> Result<Ref<'_, Vec<Cookie<'static>>>, CookieParseError> {
        self.req.cookies()
    }

    /// Return request cookie.
    #[cfg(feature = "cookies")]
    #[inline]
    pub fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        self.req.cookie(name)
    }

    /// Set request payload.
    #[inline]
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

    /// Creates a context object for use with a routing [guard](crate::guard).
    #[inline]
    pub fn guard_ctx(&self) -> GuardContext<'_> {
        GuardContext { req: self }
    }
}

impl Resource for ServiceRequest {
    type Path = Url;

    #[inline]
    fn resource_path(&mut self) -> &mut Path<Self::Path> {
        self.match_info_mut()
    }
}

impl HttpMessage for ServiceRequest {
    type Stream = BoxedPayloadStream;

    #[inline]
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    #[inline]
    fn extensions(&self) -> Ref<'_, Extensions> {
        self.req.extensions()
    }

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

/// A service level response wrapper.
pub struct ServiceResponse<B = BoxBody> {
    request: HttpRequest,
    response: HttpResponse<B>,
}

impl ServiceResponse<BoxBody> {
    /// Create service response from the error
    pub fn from_err<E: Into<Error>>(err: E, request: HttpRequest) -> Self {
        let response = HttpResponse::from_error(err);
        ServiceResponse { request, response }
    }
}

impl<B> ServiceResponse<B> {
    /// Create service response instance
    pub fn new(request: HttpRequest, response: HttpResponse<B>) -> Self {
        ServiceResponse { request, response }
    }

    /// Create service response for error
    #[inline]
    pub fn error_response<E: Into<Error>>(self, err: E) -> ServiceResponse {
        ServiceResponse::from_err(err, self.request)
    }

    /// Create service response
    #[inline]
    pub fn into_response<B1>(self, response: HttpResponse<B1>) -> ServiceResponse<B1> {
        ServiceResponse::new(self.request, response)
    }

    /// Returns reference to original request.
    #[inline]
    pub fn request(&self) -> &HttpRequest {
        &self.request
    }

    /// Returns reference to response.
    #[inline]
    pub fn response(&self) -> &HttpResponse<B> {
        &self.response
    }

    /// Returns mutable reference to response.
    #[inline]
    pub fn response_mut(&mut self) -> &mut HttpResponse<B> {
        &mut self.response
    }

    /// Returns response status code.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.response.status()
    }

    /// Returns response's headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        self.response.headers()
    }

    /// Returns mutable response's headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.response.headers_mut()
    }

    /// Destructures `ServiceResponse` into request and response components.
    #[inline]
    pub fn into_parts(self) -> (HttpRequest, HttpResponse<B>) {
        (self.request, self.response)
    }

    /// Map the current body type to another using a closure. Returns a new response.
    ///
    /// Closure receives the response head and the current body type.
    #[inline]
    pub fn map_body<F, B2>(self, f: F) -> ServiceResponse<B2>
    where
        F: FnOnce(&mut ResponseHead, B) -> B2,
    {
        let response = self.response.map_body(f);

        ServiceResponse {
            response,
            request: self.request,
        }
    }

    #[inline]
    pub fn map_into_left_body<R>(self) -> ServiceResponse<EitherBody<B, R>> {
        self.map_body(|_, body| EitherBody::left(body))
    }

    #[inline]
    pub fn map_into_right_body<L>(self) -> ServiceResponse<EitherBody<L, B>> {
        self.map_body(|_, body| EitherBody::right(body))
    }

    #[inline]
    pub fn map_into_boxed_body(self) -> ServiceResponse<BoxBody>
    where
        B: MessageBody + 'static,
    {
        self.map_body(|_, body| body.boxed())
    }

    /// Consumes the response and returns its body.
    #[inline]
    pub fn into_body(self) -> B {
        self.response.into_body()
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

impl<B> fmt::Debug for ServiceResponse<B>
where
    B: MessageBody,
    B::Error: Into<Error>,
{
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
    rdef: Patterns,
    name: Option<String>,
    guards: Vec<Box<dyn Guard>>,
}

impl WebService {
    /// Create new `WebService` instance.
    pub fn new<T: IntoPatterns>(path: T) -> Self {
        WebService {
            rdef: path.patterns(),
            name: None,
            guards: Vec::new(),
        }
    }

    /// Set service name.
    ///
    /// Name is used for URL generation.
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
    /// let app = App::new()
    ///     .service(
    ///         web::service("/app")
    ///             .guard(guard::Header("content-type", "text/plain"))
    ///             .finish(index)
    ///     );
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
    rdef: Patterns,
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
            ResourceDef::new(ensure_leading_slash(self.rdef))
        } else {
            ResourceDef::new(self.rdef)
        };

        if let Some(ref name) = self.name {
            rdef.set_name(name);
        }

        config.register_service(rdef, guards, self.srv, None)
    }
}

/// Macro to help register different types of services at the same time.
///
/// The max number of services that can be grouped together is 12 and all must implement the
/// [`HttpServiceFactory`] trait.
///
/// # Examples
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
    use actix_service::Service;
    use actix_utils::future::ok;

    use super::*;
    use crate::{
        guard, http,
        test::{self, init_service, TestRequest},
        web, App,
    };

    #[actix_rt::test]
    async fn test_service() {
        let srv =
            init_service(
                App::new().service(web::service("/test").name("test").finish(
                    |req: ServiceRequest| ok(req.into_response(HttpResponse::Ok().finish())),
                )),
            )
            .await;
        let req = TestRequest::with_uri("/test").to_request();
        let resp = srv.call(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        let srv =
            init_service(
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

    // allow deprecated App::data
    #[allow(deprecated)]
    #[actix_rt::test]
    async fn test_service_data() {
        let srv = init_service(
            App::new()
                .data(42u32)
                .service(
                    web::service("/test")
                        .name("test")
                        .finish(|req: ServiceRequest| {
                            assert_eq!(req.app_data::<web::Data<u32>>().unwrap().as_ref(), &42);
                            ok(req.into_response(HttpResponse::Ok().finish()))
                        }),
                ),
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
                |req: ServiceRequest| async { Ok(req.into_response(HttpResponse::Ok().finish())) }
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

    #[actix_rt::test]
    #[should_panic(expected = "called `Option::unwrap()` on a `None` value")]
    async fn cloning_request_panics() {
        async fn index(_name: web::Path<(String,)>) -> &'static str {
            ""
        }

        let app = test::init_service(
            App::new()
                .wrap_fn(|req, svc| {
                    let (req, pl) = req.into_parts();
                    let _req2 = req.clone();
                    let req = ServiceRequest::from_parts(req, pl);
                    svc.call(req)
                })
                .route("/", web::get().to(|| async { "" }))
                .service(web::resource("/resource1/{name}/index.html").route(web::get().to(index))),
        )
        .await;

        let req = test::TestRequest::default().to_request();
        let _res = test::call_service(&app, req).await;
    }
}

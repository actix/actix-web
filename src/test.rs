//! Various helpers for Actix applications to use during testing.
use std::cell::RefCell;
use std::rc::Rc;

use actix_http::cookie::Cookie;
use actix_http::http::header::{Header, HeaderName, IntoHeaderValue};
use actix_http::http::{HttpTryFrom, Method, StatusCode, Version};
use actix_http::test::TestRequest as HttpTestRequest;
use actix_http::{Extensions, PayloadStream, Request};
use actix_router::{Path, ResourceDef, Url};
use actix_rt::Runtime;
use actix_server_config::ServerConfig;
use actix_service::{FnService, IntoNewService, NewService, Service};
use bytes::Bytes;
use futures::future::{lazy, Future};

use crate::config::{AppConfig, AppConfigInner};
use crate::data::RouteData;
use crate::dev::{Body, Payload};
use crate::rmap::ResourceMap;
use crate::service::{ServiceRequest, ServiceResponse};
use crate::{Error, HttpRequest, HttpResponse};

thread_local! {
    static RT: RefCell<Runtime> = {
        RefCell::new(Runtime::new().unwrap())
    };
}

/// Runs the provided future, blocking the current thread until the future
/// completes.
///
/// This function can be used to synchronously block the current thread
/// until the provided `future` has resolved either successfully or with an
/// error. The result of the future is then returned from this function
/// call.
///
/// Note that this function is intended to be used only for testing purpose.
/// This function panics on nested call.
pub fn block_on<F>(f: F) -> Result<F::Item, F::Error>
where
    F: Future,
{
    RT.with(move |rt| rt.borrow_mut().block_on(f))
}

/// Runs the provided function, with runtime enabled.
///
/// Note that this function is intended to be used only for testing purpose.
/// This function panics on nested call.
pub fn run_on<F, R>(f: F) -> R
where
    F: Fn() -> R,
{
    RT.with(move |rt| rt.borrow_mut().block_on(lazy(|| Ok::<_, ()>(f()))))
        .unwrap()
}

/// Create service that always responds with `HttpResponse::Ok()`
pub fn ok_service() -> impl Service<
    Request = ServiceRequest<PayloadStream>,
    Response = ServiceResponse<Body>,
    Error = Error,
> {
    default_service(StatusCode::OK)
}

/// Create service that responds with response with specified status code
pub fn default_service(
    status_code: StatusCode,
) -> impl Service<
    Request = ServiceRequest<PayloadStream>,
    Response = ServiceResponse<Body>,
    Error = Error,
> {
    FnService::new(move |req: ServiceRequest<PayloadStream>| {
        req.into_response(HttpResponse::build(status_code).finish())
    })
}

/// This method accepts application builder instance, and constructs
/// service.
///
/// ```rust,ignore
/// use actix_service::Service;
/// use actix_web::{test, web, App, HttpResponse, http::StatusCode};
///
/// fn main() {
///     let mut app = test::init_service(
///         App::new()
///             .service(web::resource("/test").to(|| HttpResponse::Ok()))
///     );
///
///     // Create request object
///     let req = test::TestRequest::with_uri("/test").to_request();
///
///     // Execute application
///     let resp = test::block_on(app.call(req)).unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
/// }
/// ```
pub fn init_service<R, S, B, E>(
    app: R,
) -> impl Service<Request = Request, Response = ServiceResponse<B>, Error = E>
where
    R: IntoNewService<S, ServerConfig>,
    S: NewService<
        ServerConfig,
        Request = Request,
        Response = ServiceResponse<B>,
        Error = E,
    >,
    S::InitError: std::fmt::Debug,
{
    let cfg = ServerConfig::new("127.0.0.1:8080".parse().unwrap());
    block_on(app.into_new_service().new_service(&cfg)).unwrap()
}

/// Calls service and waits for response future completion.
///
/// ```rust,ignore
/// use actix_web::{test, App, HttpResponse, http::StatusCode};
/// use actix_service::Service;
///
/// fn main() {
///     let mut app = test::init_service(
///         App::new()
///             .service(web::resource("/test").to(|| HttpResponse::Ok()))
///     );
///
///     // Create request object
///     let req = test::TestRequest::with_uri("/test").to_request();
///
///     // Call application
///     let resp = test::call_succ_service(&mut app, req);
///     assert_eq!(resp.status(), StatusCode::OK);
/// }
/// ```
pub fn call_success<S, R, B, E>(app: &mut S, req: R) -> S::Response
where
    S: Service<Request = R, Response = ServiceResponse<B>, Error = E>,
    E: std::fmt::Debug,
{
    block_on(app.call(req)).unwrap()
}

/// Test `Request` builder.
///
/// For unit testing, actix provides a request builder type and a simple handler runner. TestRequest implements a builder-like pattern.
/// You can generate various types of request via TestRequest's methods:
///  * `TestRequest::to_request` creates `actix_http::Request` instance.
///  * `TestRequest::to_service` creates `ServiceRequest` instance, which is used for testing middlewares and chain adapters.
///  * `TestRequest::to_from` creates `ServiceFromRequest` instance, which is used for testing extractors.
///  * `TestRequest::to_http_request` creates `HttpRequest` instance, which is used for testing handlers.
///
/// ```rust,ignore
/// # use futures::IntoFuture;
/// use actix_web::{test, HttpRequest, HttpResponse, HttpMessage};
/// use actix_web::http::{header, StatusCode};
///
/// fn index(req: HttpRequest) -> HttpResponse {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         HttpResponse::Ok().into()
///     } else {
///         HttpResponse::BadRequest().into()
///     }
/// }
///
/// fn main() {
///     let req = test::TestRequest::with_header("content-type", "text/plain")
///         .to_http_request();
///
///     let resp = test::block_on(index(req).into_future()).unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
///
///     let req = test::TestRequest::default().to_http_request();
///     let resp = test::block_on(index(req).into_future()).unwrap();
///     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
/// }
/// ```
pub struct TestRequest {
    req: HttpTestRequest,
    rmap: ResourceMap,
    config: AppConfigInner,
    route_data: Extensions,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            config: AppConfigInner::default(),
            route_data: Extensions::new(),
        }
    }
}

#[allow(clippy::wrong_self_convention)]
impl TestRequest {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().uri(path).take(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            config: AppConfigInner::default(),
            route_data: Extensions::new(),
        }
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().set(hdr).take(),
            config: AppConfigInner::default(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            route_data: Extensions::new(),
        }
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        TestRequest {
            req: HttpTestRequest::default().header(key, value).take(),
            config: AppConfigInner::default(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            route_data: Extensions::new(),
        }
    }

    /// Create TestRequest and set method to `Method::GET`
    pub fn get() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().method(Method::GET).take(),
            config: AppConfigInner::default(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            route_data: Extensions::new(),
        }
    }

    /// Create TestRequest and set method to `Method::POST`
    pub fn post() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().method(Method::POST).take(),
            config: AppConfigInner::default(),
            rmap: ResourceMap::new(ResourceDef::new("")),
            route_data: Extensions::new(),
        }
    }

    /// Set HTTP version of this request
    pub fn version(mut self, ver: Version) -> Self {
        self.req.version(ver);
        self
    }

    /// Set HTTP method of this request
    pub fn method(mut self, meth: Method) -> Self {
        self.req.method(meth);
        self
    }

    /// Set HTTP Uri of this request
    pub fn uri(mut self, path: &str) -> Self {
        self.req.uri(path);
        self
    }

    /// Set a header
    pub fn set<H: Header>(mut self, hdr: H) -> Self {
        self.req.set(hdr);
        self
    }

    /// Set a header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        self.req.header(key, value);
        self
    }

    /// Set cookie for this request
    pub fn cookie(mut self, cookie: Cookie) -> Self {
        self.req.cookie(cookie);
        self
    }

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(mut self, data: B) -> Self {
        self.req.set_payload(data);
        self
    }

    /// Set application data. This is equivalent of `App::data()` method
    /// for testing purpose.
    pub fn app_data<T: 'static>(self, data: T) -> Self {
        self.config.extensions.borrow_mut().insert(data);
        self
    }

    /// Set route data. This is equivalent of `Route::data()` method
    /// for testing purpose.
    pub fn route_data<T: 'static>(mut self, data: T) -> Self {
        self.route_data.insert(RouteData::new(data));
        self
    }

    #[cfg(test)]
    /// Set request config
    pub(crate) fn rmap(mut self, rmap: ResourceMap) -> Self {
        self.rmap = rmap;
        self
    }

    /// Complete request creation and generate `Request` instance
    pub fn to_request(mut self) -> Request<PayloadStream> {
        self.req.finish()
    }

    /// Complete request creation and generate `ServiceRequest` instance
    pub fn to_srv_request(mut self) -> ServiceRequest<PayloadStream> {
        let req = self.req.finish();

        ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.rmap),
            AppConfig::new(self.config),
        )
    }

    /// Complete request creation and generate `ServiceResponse` instance
    pub fn to_srv_response<B>(self, res: HttpResponse<B>) -> ServiceResponse<B> {
        self.to_srv_request().into_response(res)
    }

    /// Complete request creation and generate `HttpRequest` instance
    pub fn to_http_request(mut self) -> HttpRequest {
        let req = self.req.finish();

        let mut req = ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.rmap),
            AppConfig::new(self.config),
        )
        .into_parts()
        .0;
        req.set_route_data(Some(Rc::new(self.route_data)));
        req
    }

    /// Complete request creation and generate `HttpRequest` and `Payload` instances
    pub fn to_http_parts(mut self) -> (HttpRequest, Payload) {
        let req = self.req.finish();

        let (mut req, pl) = ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.rmap),
            AppConfig::new(self.config),
        )
        .into_parts();

        req.set_route_data(Some(Rc::new(self.route_data)));
        (req, pl)
    }

    /// Runs the provided future, blocking the current thread until the future
    /// completes.
    ///
    /// This function can be used to synchronously block the current thread
    /// until the provided `future` has resolved either successfully or with an
    /// error. The result of the future is then returned from this function
    /// call.
    ///
    /// Note that this function is intended to be used only for testing purpose.
    /// This function panics on nested call.
    pub fn block_on<F>(f: F) -> Result<F::Item, F::Error>
    where
        F: Future,
    {
        block_on(f)
    }
}

//! Various helpers for Actix applications to use during testing.
use std::cell::RefCell;
use std::rc::Rc;

use actix_http::http::header::{Header, HeaderName, IntoHeaderValue};
use actix_http::http::{HttpTryFrom, Method, Version};
use actix_http::test::TestRequest as HttpTestRequest;
use actix_http::{Extensions, PayloadStream, Request};
use actix_router::{Path, Url};
use actix_rt::Runtime;
use actix_service::{IntoNewService, NewService, Service};
use bytes::Bytes;
use futures::Future;

use crate::request::HttpRequest;
use crate::service::{ServiceFromRequest, ServiceRequest, ServiceResponse};

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

/// This method accepts application builder instance, and constructs
/// service.
///
/// ```rust,ignore
/// use actix_web::{test, App, HttpResponse, http::StatusCode};
/// use actix_service::Service;
///
/// fn main() {
///     let mut app = test::init_service(
///         App::new()
///             .resource("/test", |r| r.to(|| HttpResponse::Ok()))
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
) -> impl Service<Request, Response = ServiceResponse<B>, Error = E>
where
    R: IntoNewService<S, Request, ()>,
    S: NewService<Request, Response = ServiceResponse<B>, Error = E>,
    S::InitError: std::fmt::Debug,
{
    block_on(app.into_new_service().new_service(&())).unwrap()
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
///             .resource("/test", |r| r.to(|| HttpResponse::Ok()))
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
    S: Service<R, Response = ServiceResponse<B>, Error = E>,
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
    extensions: Extensions,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default(),
            extensions: Extensions::new(),
        }
    }
}

impl TestRequest {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().uri(path).take(),
            extensions: Extensions::new(),
        }
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().set(hdr).take(),
            extensions: Extensions::new(),
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
            extensions: Extensions::new(),
        }
    }

    /// Create TestRequest and set method to `Method::GET`
    pub fn get() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().method(Method::GET).take(),
            extensions: Extensions::new(),
        }
    }

    /// Create TestRequest and set method to `Method::POST`
    pub fn post() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default().method(Method::POST).take(),
            extensions: Extensions::new(),
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

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(mut self, data: B) -> Self {
        self.req.set_payload(data);
        self
    }

    /// Set request config
    pub fn config<T: 'static>(mut self, data: T) -> Self {
        self.extensions.insert(data);
        self
    }

    /// Complete request creation and generate `ServiceRequest` instance
    pub fn to_service(mut self) -> ServiceRequest<PayloadStream> {
        let req = self.req.finish();

        ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.extensions),
        )
    }

    /// Complete request creation and generate `Request` instance
    pub fn to_request(mut self) -> Request<PayloadStream> {
        self.req.finish()
    }

    /// Complete request creation and generate `HttpRequest` instance
    pub fn to_http_request(mut self) -> HttpRequest {
        let req = self.req.finish();

        ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.extensions),
        )
        .into_request()
    }

    /// Complete request creation and generate `ServiceFromRequest` instance
    pub fn to_from(mut self) -> ServiceFromRequest<PayloadStream> {
        let req = self.req.finish();

        let req = ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.extensions),
        );
        ServiceFromRequest::new(req, None)
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

//! Various helpers for Actix applications to use during testing.
use std::str::FromStr;

use bytes::Bytes;
use futures::IntoFuture;
use tokio_current_thread::Runtime;

use actix_http::dev::Payload;
use actix_http::http::header::{Header, HeaderName, IntoHeaderValue};
use actix_http::http::{HeaderMap, HttpTryFrom, Method, Uri, Version};
use actix_http::Request as HttpRequest;
use actix_router::{Path, Url};

use crate::app::State;
use crate::request::Request;
use crate::service::ServiceRequest;

/// Test `Request` builder
///
/// ```rust,ignore
/// # extern crate http;
/// # extern crate actix_web;
/// # use http::{header, StatusCode};
/// # use actix_web::*;
/// use actix_web::test::TestRequest;
///
/// fn index(req: &HttpRequest) -> HttpResponse {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         HttpResponse::Ok().into()
///     } else {
///         HttpResponse::BadRequest().into()
///     }
/// }
///
/// fn main() {
///     let resp = TestRequest::with_header("content-type", "text/plain")
///         .run(&index)
///         .unwrap();
///     assert_eq!(resp.status(), StatusCode::OK);
///
///     let resp = TestRequest::default().run(&index).unwrap();
///     assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
/// }
/// ```
pub struct TestRequest<S> {
    state: S,
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    params: Path<Url>,
    payload: Option<Payload>,
}

impl Default for TestRequest<()> {
    fn default() -> TestRequest<()> {
        TestRequest {
            state: (),
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            params: Path::new(Url::default()),
            payload: None,
        }
    }
}

impl TestRequest<()> {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest<()> {
        TestRequest::default().uri(path)
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest<()> {
        TestRequest::default().set(hdr)
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest<()>
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        TestRequest::default().header(key, value)
    }
}

impl<S: 'static> TestRequest<S> {
    /// Start HttpRequest build process with application state
    pub fn with_state(state: S) -> TestRequest<S> {
        TestRequest {
            state,
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            params: Path::new(Url::default()),
            payload: None,
        }
    }

    /// Set HTTP version of this request
    pub fn version(mut self, ver: Version) -> Self {
        self.version = ver;
        self
    }

    /// Set HTTP method of this request
    pub fn method(mut self, meth: Method) -> Self {
        self.method = meth;
        self
    }

    /// Set HTTP Uri of this request
    pub fn uri(mut self, path: &str) -> Self {
        self.uri = Uri::from_str(path).unwrap();
        self
    }

    /// Set a header
    pub fn set<H: Header>(mut self, hdr: H) -> Self {
        if let Ok(value) = hdr.try_into() {
            self.headers.append(H::name(), value);
            return self;
        }
        panic!("Can not set header");
    }

    /// Set a header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Ok(key) = HeaderName::try_from(key) {
            if let Ok(value) = value.try_into() {
                self.headers.append(key, value);
                return self;
            }
        }
        panic!("Can not create header");
    }

    /// Set request path pattern parameter
    pub fn param(mut self, name: &'static str, value: &'static str) -> Self {
        self.params.add_static(name, value);
        self
    }

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(mut self, data: B) -> Self {
        let mut payload = Payload::empty();
        payload.unread_data(data.into());
        self.payload = Some(payload);
        self
    }

    /// Complete request creation and generate `HttpRequest` instance
    pub fn finish(self) -> ServiceRequest<S> {
        let TestRequest {
            state,
            method,
            uri,
            version,
            headers,
            mut params,
            payload,
        } = self;

        params.get_mut().update(&uri);

        let mut req = HttpRequest::new();
        {
            let inner = req.inner_mut();
            inner.head.uri = uri;
            inner.head.method = method;
            inner.head.version = version;
            inner.head.headers = headers;
            *inner.payload.borrow_mut() = payload;
        }

        Request::new(State::new(state), req, params)
    }

    /// This method generates `HttpRequest` instance and executes handler
    pub fn run_async<F, R, I, E>(self, f: F) -> Result<I, E>
    where
        F: FnOnce(&Request<S>) -> R,
        R: IntoFuture<Item = I, Error = E>,
    {
        let mut rt = Runtime::new().unwrap();
        rt.block_on(f(&self.finish()).into_future())
    }
}

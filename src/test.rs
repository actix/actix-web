//! Various helpers for Actix applications to use during testing.
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use actix_http::http::header::{Header, HeaderName, IntoHeaderValue};
use actix_http::http::{HttpTryFrom, Method, Version};
use actix_http::test::TestRequest;
use actix_http::{Extensions, PayloadStream};
use actix_router::{Path, Url};
use bytes::Bytes;

use crate::request::HttpRequest;
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
pub struct TestServiceRequest {
    req: TestRequest,
    extensions: Extensions,
}

impl Default for TestServiceRequest {
    fn default() -> TestServiceRequest {
        TestServiceRequest {
            req: TestRequest::default(),
            extensions: Extensions::new(),
        }
    }
}

impl TestServiceRequest {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestServiceRequest {
        TestServiceRequest {
            req: TestRequest::default().uri(path).take(),
            extensions: Extensions::new(),
        }
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestServiceRequest {
        TestServiceRequest {
            req: TestRequest::default().set(hdr).take(),
            extensions: Extensions::new(),
        }
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestServiceRequest
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        TestServiceRequest {
            req: TestRequest::default().header(key, value).take(),
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

    /// Complete request creation and generate `ServiceRequest` instance
    pub fn finish(mut self) -> ServiceRequest<PayloadStream> {
        let req = self.req.finish();

        ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.extensions),
        )
    }

    /// Complete request creation and generate `HttpRequest` instance
    pub fn request(mut self) -> HttpRequest {
        let req = self.req.finish();

        ServiceRequest::new(
            Path::new(Url::new(req.uri().clone())),
            req,
            Rc::new(self.extensions),
        )
        .into_request()
    }
}

impl Deref for TestServiceRequest {
    type Target = TestRequest;

    fn deref(&self) -> &TestRequest {
        &self.req
    }
}

impl DerefMut for TestServiceRequest {
    fn deref_mut(&mut self) -> &mut TestRequest {
        &mut self.req
    }
}

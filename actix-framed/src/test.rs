//! Various helpers for Actix applications to use during testing.
use std::convert::TryFrom;
use std::future::Future;

use actix_codec::Framed;
use actix_http::h1::Codec;
use actix_http::http::header::{Header, HeaderName, IntoHeaderValue};
use actix_http::http::{Error as HttpError, Method, Uri, Version};
use actix_http::test::{TestBuffer, TestRequest as HttpTestRequest};
use actix_router::{Path, Url};

use crate::{FramedRequest, State};

/// Test `Request` builder.
pub struct TestRequest<S = ()> {
    req: HttpTestRequest,
    path: Path<Url>,
    state: State<S>,
}

impl Default for TestRequest<()> {
    fn default() -> TestRequest {
        TestRequest {
            req: HttpTestRequest::default(),
            path: Path::new(Url::new(Uri::default())),
            state: State::new(()),
        }
    }
}

impl TestRequest<()> {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> Self {
        Self::get().uri(path)
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> Self {
        Self::default().set(hdr)
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        Self::default().header(key, value)
    }

    /// Create TestRequest and set method to `Method::GET`
    pub fn get() -> Self {
        Self::default().method(Method::GET)
    }

    /// Create TestRequest and set method to `Method::POST`
    pub fn post() -> Self {
        Self::default().method(Method::POST)
    }
}

impl<S> TestRequest<S> {
    /// Create TestRequest and set request uri
    pub fn with_state(state: S) -> TestRequest<S> {
        let req = TestRequest::get();
        TestRequest {
            state: State::new(state),
            req: req.req,
            path: req.path,
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
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        V: IntoHeaderValue,
    {
        self.req.header(key, value);
        self
    }

    /// Set request path pattern parameter
    pub fn param(mut self, name: &'static str, value: &'static str) -> Self {
        self.path.add_static(name, value);
        self
    }

    /// Complete request creation and generate `Request` instance
    pub fn finish(mut self) -> FramedRequest<TestBuffer, S> {
        let req = self.req.finish();
        self.path.get_mut().update(req.uri());
        let framed = Framed::new(TestBuffer::empty(), Codec::default());
        FramedRequest::new(req, framed, self.path, self.state)
    }

    /// This method generates `FramedRequest` instance and executes async handler
    pub async fn run<F, R, I, E>(self, f: F) -> Result<I, E>
    where
        F: FnOnce(FramedRequest<TestBuffer, S>) -> R,
        R: Future<Output = Result<I, E>>,
    {
        f(self.finish()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test() {
        let req = TestRequest::with_uri("/index.html")
            .header("x-test", "test")
            .param("test", "123")
            .finish();

        assert_eq!(*req.state(), ());
        assert_eq!(req.version(), Version::HTTP_11);
        assert_eq!(req.method(), Method::GET);
        assert_eq!(req.path(), "/index.html");
        assert_eq!(req.query_string(), "");
        assert_eq!(
            req.headers().get("x-test").unwrap().to_str().unwrap(),
            "test"
        );
        assert_eq!(&req.match_info()["test"], "123");
    }
}

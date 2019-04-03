//! Test Various helpers for Actix applications to use during testing.
use std::fmt::Write as FmtWrite;
use std::str::FromStr;

use bytes::Bytes;
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, HttpTryFrom, Method, Uri, Version};
use percent_encoding::{percent_encode, USERINFO_ENCODE_SET};

use crate::cookie::{Cookie, CookieJar};
use crate::header::{Header, IntoHeaderValue};
use crate::payload::Payload;
use crate::Request;

/// Test `Request` builder
///
/// ```rust,ignore
/// # extern crate http;
/// # extern crate actix_web;
/// # use http::{header, StatusCode};
/// # use actix_web::*;
/// use actix_web::test::TestRequest;
///
/// fn index(req: &HttpRequest) -> Response {
///     if let Some(hdr) = req.headers().get(header::CONTENT_TYPE) {
///         Response::Ok().into()
///     } else {
///         Response::BadRequest().into()
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
pub struct TestRequest(Option<Inner>);

struct Inner {
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    cookies: CookieJar,
    payload: Option<Payload>,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest(Some(Inner {
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            cookies: CookieJar::new(),
            payload: None,
        }))
    }
}

impl TestRequest {
    /// Create TestRequest and set request uri
    pub fn with_uri(path: &str) -> TestRequest {
        TestRequest::default().uri(path).take()
    }

    /// Create TestRequest and set header
    pub fn with_hdr<H: Header>(hdr: H) -> TestRequest {
        TestRequest::default().set(hdr).take()
    }

    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> TestRequest
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        TestRequest::default().header(key, value).take()
    }

    /// Set HTTP version of this request
    pub fn version(&mut self, ver: Version) -> &mut Self {
        parts(&mut self.0).version = ver;
        self
    }

    /// Set HTTP method of this request
    pub fn method(&mut self, meth: Method) -> &mut Self {
        parts(&mut self.0).method = meth;
        self
    }

    /// Set HTTP Uri of this request
    pub fn uri(&mut self, path: &str) -> &mut Self {
        parts(&mut self.0).uri = Uri::from_str(path).unwrap();
        self
    }

    /// Set a header
    pub fn set<H: Header>(&mut self, hdr: H) -> &mut Self {
        if let Ok(value) = hdr.try_into() {
            parts(&mut self.0).headers.append(H::name(), value);
            return self;
        }
        panic!("Can not set header");
    }

    /// Set a header
    pub fn header<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Ok(key) = HeaderName::try_from(key) {
            if let Ok(value) = value.try_into() {
                parts(&mut self.0).headers.append(key, value);
                return self;
            }
        }
        panic!("Can not create header");
    }

    /// Set cookie for this request
    pub fn cookie<'a>(&mut self, cookie: Cookie<'a>) -> &mut Self {
        parts(&mut self.0).cookies.add(cookie.into_owned());
        self
    }

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(&mut self, data: B) -> &mut Self {
        let mut payload = crate::h1::Payload::empty();
        payload.unread_data(data.into());
        parts(&mut self.0).payload = Some(payload.into());
        self
    }

    pub fn take(&mut self) -> TestRequest {
        TestRequest(self.0.take())
    }

    /// Complete request creation and generate `Request` instance
    pub fn finish(&mut self) -> Request {
        let inner = self.0.take().expect("cannot reuse test request builder");;

        let mut req = if let Some(pl) = inner.payload {
            Request::with_payload(pl)
        } else {
            Request::with_payload(crate::h1::Payload::empty().into())
        };

        let head = req.head_mut();
        head.uri = inner.uri;
        head.method = inner.method;
        head.version = inner.version;
        head.headers = inner.headers;

        let mut cookie = String::new();
        for c in inner.cookies.delta() {
            let name = percent_encode(c.name().as_bytes(), USERINFO_ENCODE_SET);
            let value = percent_encode(c.value().as_bytes(), USERINFO_ENCODE_SET);
            let _ = write!(&mut cookie, "; {}={}", name, value);
        }
        if !cookie.is_empty() {
            head.headers.insert(
                header::COOKIE,
                HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
            );
        }

        req
    }
}

#[inline]
fn parts<'a>(parts: &'a mut Option<Inner>) -> &'a mut Inner {
    parts.as_mut().expect("cannot reuse test request builder")
}

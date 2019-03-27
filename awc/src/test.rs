//! Test Various helpers for Actix applications to use during testing.

use actix_http::http::header::{Header, IntoHeaderValue};
use actix_http::http::{HeaderName, HttpTryFrom, Version};
use actix_http::{h1, Payload, ResponseHead};
use bytes::Bytes;
#[cfg(feature = "cookies")]
use cookie::{Cookie, CookieJar};

use crate::ClientResponse;

/// Test `ClientResponse` builder
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
pub struct TestResponse(Option<Inner>);

struct Inner {
    head: ResponseHead,
    #[cfg(feature = "cookies")]
    cookies: CookieJar,
    payload: Option<Payload>,
}

impl Default for TestResponse {
    fn default() -> TestResponse {
        TestResponse(Some(Inner {
            head: ResponseHead::default(),
            #[cfg(feature = "cookies")]
            cookies: CookieJar::new(),
            payload: None,
        }))
    }
}

impl TestResponse {
    /// Create TestRequest and set header
    pub fn with_header<K, V>(key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        Self::default().header(key, value).take()
    }

    /// Set HTTP version of this request
    pub fn version(&mut self, ver: Version) -> &mut Self {
        parts(&mut self.0).head.version = ver;
        self
    }

    /// Set a header
    pub fn set<H: Header>(&mut self, hdr: H) -> &mut Self {
        if let Ok(value) = hdr.try_into() {
            parts(&mut self.0).head.headers.append(H::name(), value);
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
                parts(&mut self.0).head.headers.append(key, value);
                return self;
            }
        }
        panic!("Can not create header");
    }

    /// Set cookie for this request
    #[cfg(feature = "cookies")]
    pub fn cookie<'a>(&mut self, cookie: Cookie<'a>) -> &mut Self {
        parts(&mut self.0).cookies.add(cookie.into_owned());
        self
    }

    /// Set request payload
    pub fn set_payload<B: Into<Bytes>>(&mut self, data: B) -> &mut Self {
        let mut payload = h1::Payload::empty();
        payload.unread_data(data.into());
        parts(&mut self.0).payload = Some(payload.into());
        self
    }

    pub fn take(&mut self) -> Self {
        Self(self.0.take())
    }

    /// Complete request creation and generate `Request` instance
    pub fn finish(&mut self) -> ClientResponse {
        let inner = self.0.take().expect("cannot reuse test request builder");;
        let mut head = inner.head;

        #[cfg(feature = "cookies")]
        {
            use std::fmt::Write as FmtWrite;

            use actix_http::http::header::{self, HeaderValue};
            use percent_encoding::{percent_encode, USERINFO_ENCODE_SET};

            let mut cookie = String::new();
            for c in inner.cookies.delta() {
                let name = percent_encode(c.name().as_bytes(), USERINFO_ENCODE_SET);
                let value = percent_encode(c.value().as_bytes(), USERINFO_ENCODE_SET);
                let _ = write!(&mut cookie, "; {}={}", name, value);
            }
            if !cookie.is_empty() {
                head.headers.insert(
                    header::SET_COOKIE,
                    HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
                );
            }
        }

        if let Some(pl) = inner.payload {
            ClientResponse::new(head, pl)
        } else {
            ClientResponse::new(head, h1::Payload::empty().into())
        }
    }
}

#[inline]
fn parts<'a>(parts: &'a mut Option<Inner>) -> &'a mut Inner {
    parts.as_mut().expect("cannot reuse test request builder")
}

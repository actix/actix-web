//! Test Various helpers for Actix applications to use during testing.

use actix_http::http::header::{Header, IntoHeaderValue};
use actix_http::http::{HeaderName, HttpTryFrom, Version};
use actix_http::{h1, Payload, ResponseHead};
use bytes::Bytes;
#[cfg(feature = "cookies")]
use cookie::{Cookie, CookieJar};

use crate::ClientResponse;

/// Test `ClientResponse` builder
pub struct TestResponse {
    head: ResponseHead,
    #[cfg(feature = "cookies")]
    cookies: CookieJar,
    payload: Option<Payload>,
}

impl Default for TestResponse {
    fn default() -> TestResponse {
        TestResponse {
            head: ResponseHead::default(),
            #[cfg(feature = "cookies")]
            cookies: CookieJar::new(),
            payload: None,
        }
    }
}

impl TestResponse {
    /// Create TestResponse and set header
    pub fn with_header<K, V>(key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        Self::default().header(key, value)
    }

    /// Set HTTP version of this response
    pub fn version(mut self, ver: Version) -> Self {
        self.head.version = ver;
        self
    }

    /// Set a header
    pub fn set<H: Header>(mut self, hdr: H) -> Self {
        if let Ok(value) = hdr.try_into() {
            self.head.headers.append(H::name(), value);
            return self;
        }
        panic!("Can not set header");
    }

    /// Append a header
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        HeaderName: HttpTryFrom<K>,
        V: IntoHeaderValue,
    {
        if let Ok(key) = HeaderName::try_from(key) {
            if let Ok(value) = value.try_into() {
                self.head.headers.append(key, value);
                return self;
            }
        }
        panic!("Can not create header");
    }

    /// Set cookie for this response
    #[cfg(feature = "cookies")]
    pub fn cookie<'a>(mut self, cookie: Cookie<'a>) -> Self {
        self.cookies.add(cookie.into_owned());
        self
    }

    /// Set response's payload
    pub fn set_payload<B: Into<Bytes>>(mut self, data: B) -> Self {
        let mut payload = h1::Payload::empty();
        payload.unread_data(data.into());
        self.payload = Some(payload.into());
        self
    }

    /// Complete response creation and generate `ClientResponse` instance
    pub fn finish(self) -> ClientResponse {
        let mut head = self.head;

        #[cfg(feature = "cookies")]
        {
            use std::fmt::Write as FmtWrite;

            use actix_http::http::header::{self, HeaderValue};
            use percent_encoding::{percent_encode, USERINFO_ENCODE_SET};

            let mut cookie = String::new();
            for c in self.cookies.delta() {
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

        if let Some(pl) = self.payload {
            ClientResponse::new(head, pl)
        } else {
            ClientResponse::new(head, h1::Payload::empty().into())
        }
    }
}

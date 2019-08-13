//! Test helpers for actix http client to use during testing.
use std::fmt::Write as FmtWrite;

use actix_http::cookie::{Cookie, CookieJar, USERINFO};
use actix_http::http::header::{self, Header, HeaderValue, IntoHeaderValue};
use actix_http::http::{HeaderName, HttpTryFrom, StatusCode, Version};
use actix_http::{h1, Payload, ResponseHead};
use bytes::Bytes;
use percent_encoding::percent_encode;

use crate::ClientResponse;

/// Test `ClientResponse` builder
pub struct TestResponse {
    head: ResponseHead,
    cookies: CookieJar,
    payload: Option<Payload>,
}

impl Default for TestResponse {
    fn default() -> TestResponse {
        TestResponse {
            head: ResponseHead::new(StatusCode::OK),
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
    pub fn cookie(mut self, cookie: Cookie<'_>) -> Self {
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

        let mut cookie = String::new();
        for c in self.cookies.delta() {
            let name = percent_encode(c.name().as_bytes(), USERINFO);
            let value = percent_encode(c.value().as_bytes(), USERINFO);
            let _ = write!(&mut cookie, "; {}={}", name, value);
        }
        if !cookie.is_empty() {
            head.headers.insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie.as_str()[2..]).unwrap(),
            );
        }

        if let Some(pl) = self.payload {
            ClientResponse::new(head, pl)
        } else {
            ClientResponse::new(head, h1::Payload::empty().into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::{cookie, http::header};

    #[test]
    fn test_basics() {
        let res = TestResponse::default()
            .version(Version::HTTP_2)
            .set(header::Date(SystemTime::now().into()))
            .cookie(cookie::Cookie::build("name", "value").finish())
            .finish();
        assert!(res.headers().contains_key(header::SET_COOKIE));
        assert!(res.headers().contains_key(header::DATE));
        assert_eq!(res.version(), Version::HTTP_2);
    }
}

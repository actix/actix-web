//! Test helpers for actix http client to use during testing.

use actix_http::{h1, header::TryIntoHeaderPair, Payload, ResponseHead, StatusCode, Version};
use bytes::Bytes;

#[cfg(feature = "cookies")]
use crate::cookie::{Cookie, CookieJar};
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
            head: ResponseHead::new(StatusCode::OK),
            #[cfg(feature = "cookies")]
            cookies: CookieJar::new(),
            payload: None,
        }
    }
}

impl TestResponse {
    /// Create TestResponse and set header
    pub fn with_header(header: impl TryIntoHeaderPair) -> Self {
        Self::default().insert_header(header)
    }

    /// Set HTTP version of this response
    pub fn version(mut self, ver: Version) -> Self {
        self.head.version = ver;
        self
    }

    /// Insert a header
    pub fn insert_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        if let Ok((key, value)) = header.try_into_pair() {
            self.head.headers.insert(key, value);
            return self;
        }
        panic!("Can not set header");
    }

    /// Append a header
    pub fn append_header(mut self, header: impl TryIntoHeaderPair) -> Self {
        if let Ok((key, value)) = header.try_into_pair() {
            self.head.headers.append(key, value);
            return self;
        }
        panic!("Can not create header");
    }

    /// Set cookie for this response
    #[cfg(feature = "cookies")]
    pub fn cookie(mut self, cookie: Cookie<'_>) -> Self {
        self.cookies.add(cookie.into_owned());
        self
    }

    /// Set response's payload
    pub fn set_payload<B: Into<Bytes>>(mut self, data: B) -> Self {
        let (_, mut payload) = h1::Payload::create(true);
        payload.unread_data(data.into());
        self.payload = Some(payload.into());
        self
    }

    /// Complete response creation and generate `ClientResponse` instance
    pub fn finish(self) -> ClientResponse {
        // allow unused mut when cookies feature is disabled
        #[allow(unused_mut)]
        let mut head = self.head;

        #[cfg(feature = "cookies")]
        for cookie in self.cookies.delta() {
            use actix_http::header::{self, HeaderValue};

            head.headers.insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie.encoded().to_string()).unwrap(),
            );
        }

        if let Some(pl) = self.payload {
            ClientResponse::new(head, pl)
        } else {
            let (_, payload) = h1::Payload::create(true);
            ClientResponse::new(head, payload.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use actix_http::header::HttpDate;

    use super::*;
    use crate::http::header;

    #[test]
    fn test_basics() {
        let res = TestResponse::default()
            .version(Version::HTTP_2)
            .insert_header((header::DATE, HttpDate::from(SystemTime::now())))
            .cookie(cookie::Cookie::build("name", "value").finish())
            .finish();
        assert!(res.headers().contains_key(header::SET_COOKIE));
        assert!(res.headers().contains_key(header::DATE));
        assert_eq!(res.version(), Version::HTTP_2);
    }
}

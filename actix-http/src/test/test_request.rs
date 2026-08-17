use std::str::FromStr;

use bytes::Bytes;
use http::{header, Method, Uri, Version};

use crate::{
    header::{HeaderMap, TryIntoHeaderPair},
    payload::Payload,
    Request,
};

/// Test `Request` builder.
pub struct TestRequest(Option<Inner>);

struct Inner {
    version: Version,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    payload: Option<Payload>,
}

impl Default for TestRequest {
    fn default() -> TestRequest {
        TestRequest(Some(Inner {
            method: Method::GET,
            uri: Uri::from_str("/").unwrap(),
            version: Version::HTTP_11,
            headers: HeaderMap::new(),
            payload: None,
        }))
    }
}

impl TestRequest {
    /// Create a default TestRequest and then set its URI.
    pub fn with_uri(path: &str) -> TestRequest {
        TestRequest::default().uri(path).take()
    }

    /// Set HTTP version of this request.
    pub fn version(&mut self, ver: Version) -> &mut Self {
        parts(&mut self.0).version = ver;
        self
    }

    /// Set HTTP method of this request.
    pub fn method(&mut self, meth: Method) -> &mut Self {
        parts(&mut self.0).method = meth;
        self
    }

    /// Set URI of this request.
    ///
    /// # Panics
    /// If provided URI is invalid.
    pub fn uri(&mut self, path: &str) -> &mut Self {
        parts(&mut self.0).uri = Uri::from_str(path).unwrap();
        self
    }

    /// Insert a header, replacing any that were set with an equivalent field name.
    pub fn insert_header(&mut self, header: impl TryIntoHeaderPair) -> &mut Self {
        match header.try_into_pair() {
            Ok((key, value)) => {
                parts(&mut self.0).headers.insert(key, value);
            }
            Err(err) => {
                panic!("Error inserting test header: {}.", err.into());
            }
        }

        self
    }

    /// Append a header, keeping any that were set with an equivalent field name.
    pub fn append_header(&mut self, header: impl TryIntoHeaderPair) -> &mut Self {
        match header.try_into_pair() {
            Ok((key, value)) => {
                parts(&mut self.0).headers.append(key, value);
            }
            Err(err) => {
                panic!("Error inserting test header: {}.", err.into());
            }
        }

        self
    }

    /// Set request payload.
    ///
    /// This sets the `Content-Length` header with the size of `data`.
    pub fn set_payload(&mut self, data: impl Into<Bytes>) -> &mut Self {
        let mut payload = crate::h1::Payload::empty();
        let bytes = data.into();
        self.insert_header((header::CONTENT_LENGTH, bytes.len()));
        payload.unread_data(bytes);
        parts(&mut self.0).payload = Some(payload.into());
        self
    }

    pub fn take(&mut self) -> TestRequest {
        TestRequest(self.0.take())
    }

    /// Complete request creation and generate `Request` instance.
    pub fn finish(&mut self) -> Request {
        let inner = self.0.take().expect("cannot reuse test request builder");

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

        req
    }
}

#[inline]
fn parts(parts: &mut Option<Inner>) -> &mut Inner {
    parts.as_mut().expect("cannot reuse test request builder")
}

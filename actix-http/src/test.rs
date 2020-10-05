//! Test Various helpers for Actix applications to use during testing.
use std::convert::TryFrom;
use std::io::{self, Read, Write};
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll};

use actix_codec::{AsyncRead, AsyncWrite};
use bytes::{Bytes, BytesMut};
use http::header::{self, HeaderName, HeaderValue};
use http::{Error as HttpError, Method, Uri, Version};

use crate::cookie::{Cookie, CookieJar};
use crate::header::HeaderMap;
use crate::header::{Header, IntoHeaderValue};
use crate::payload::Payload;
use crate::Request;

/// Test `Request` builder
///
/// ```rust,ignore
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
/// let resp = TestRequest::with_header("content-type", "text/plain")
///     .run(&index)
///     .unwrap();
/// assert_eq!(resp.status(), StatusCode::OK);
///
/// let resp = TestRequest::default().run(&index).unwrap();
/// assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
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
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
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
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
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

        let cookie: String = inner
            .cookies
            .delta()
            // ensure only name=value is written to cookie header
            .map(|c| Cookie::new(c.name(), c.value()).encoded().to_string())
            .collect::<Vec<_>>()
            .join("; ");

        if !cookie.is_empty() {
            head.headers
                .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        }

        req
    }
}

#[inline]
fn parts(parts: &mut Option<Inner>) -> &mut Inner {
    parts.as_mut().expect("cannot reuse test request builder")
}

/// Async io buffer
pub struct TestBuffer {
    pub read_buf: BytesMut,
    pub write_buf: BytesMut,
    pub err: Option<io::Error>,
}

impl TestBuffer {
    /// Create new TestBuffer instance
    pub fn new<T>(data: T) -> TestBuffer
    where
        BytesMut: From<T>,
    {
        TestBuffer {
            read_buf: BytesMut::from(data),
            write_buf: BytesMut::new(),
            err: None,
        }
    }

    /// Create new empty TestBuffer instance
    pub fn empty() -> TestBuffer {
        TestBuffer::new("")
    }

    /// Add extra data to read buffer.
    pub fn extend_read_buf<T: AsRef<[u8]>>(&mut self, data: T) {
        self.read_buf.extend_from_slice(data.as_ref())
    }
}

impl io::Read for TestBuffer {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
        if self.read_buf.is_empty() {
            if self.err.is_some() {
                Err(self.err.take().unwrap())
            } else {
                Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
            }
        } else {
            let size = std::cmp::min(self.read_buf.len(), dst.len());
            let b = self.read_buf.split_to(size);
            dst[..size].copy_from_slice(&b);
            Ok(size)
        }
    }
}

impl io::Write for TestBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_buf.extend(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncRead for TestBuffer {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(self.get_mut().read(buf))
    }
}

impl AsyncWrite for TestBuffer {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(self.get_mut().write(buf))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

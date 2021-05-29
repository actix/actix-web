//! Various testing helpers for use in internal and app tests.

use std::{
    cell::{Ref, RefCell},
    io::{self, Read, Write},
    pin::Pin,
    rc::Rc,
    str::FromStr,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use bytes::{Bytes, BytesMut};
use http::{Method, Uri, Version};

use crate::{
    header::{HeaderMap, IntoHeaderPair},
    payload::Payload,
    Request,
};

/// Test `Request` builder
///
/// ```ignore
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
/// let resp = TestRequest::default().insert_header("content-type", "text/plain")
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
    pub fn insert_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        match header.try_into_header_pair() {
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
    pub fn append_header<H>(&mut self, header: H) -> &mut Self
    where
        H: IntoHeaderPair,
    {
        match header.try_into_header_pair() {
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
    pub fn set_payload<B: Into<Bytes>>(&mut self, data: B) -> &mut Self {
        let mut payload = crate::h1::Payload::empty();
        payload.unread_data(data.into());
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

/// Async I/O test buffer.
pub struct TestBuffer {
    pub read_buf: BytesMut,
    pub write_buf: BytesMut,
    pub err: Option<io::Error>,
}

impl TestBuffer {
    /// Create new `TestBuffer` instance with initial read buffer.
    pub fn new<T>(data: T) -> Self
    where
        T: Into<BytesMut>,
    {
        Self {
            read_buf: data.into(),
            write_buf: BytesMut::new(),
            err: None,
        }
    }

    /// Create new empty `TestBuffer` instance.
    pub fn empty() -> Self {
        Self::new("")
    }

    /// Add data to read buffer.
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
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let dst = buf.initialize_unfilled();
        let res = self.get_mut().read(dst).map(|n| buf.advance(n));
        Poll::Ready(res)
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

/// Async I/O test buffer with ability to incrementally add to the read buffer.
#[derive(Clone)]
pub struct TestSeqBuffer(Rc<RefCell<TestSeqInner>>);

impl TestSeqBuffer {
    /// Create new `TestBuffer` instance with initial read buffer.
    pub fn new<T>(data: T) -> Self
    where
        T: Into<BytesMut>,
    {
        Self(Rc::new(RefCell::new(TestSeqInner {
            read_buf: data.into(),
            write_buf: BytesMut::new(),
            err: None,
        })))
    }

    /// Create new empty `TestBuffer` instance.
    pub fn empty() -> Self {
        Self::new("")
    }

    pub fn read_buf(&self) -> Ref<'_, BytesMut> {
        Ref::map(self.0.borrow(), |inner| &inner.read_buf)
    }

    pub fn write_buf(&self) -> Ref<'_, BytesMut> {
        Ref::map(self.0.borrow(), |inner| &inner.write_buf)
    }

    pub fn err(&self) -> Ref<'_, Option<io::Error>> {
        Ref::map(self.0.borrow(), |inner| &inner.err)
    }

    /// Add data to read buffer.
    pub fn extend_read_buf<T: AsRef<[u8]>>(&mut self, data: T) {
        self.0
            .borrow_mut()
            .read_buf
            .extend_from_slice(data.as_ref())
    }
}

pub struct TestSeqInner {
    read_buf: BytesMut,
    write_buf: BytesMut,
    err: Option<io::Error>,
}

impl io::Read for TestSeqBuffer {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
        if self.0.borrow().read_buf.is_empty() {
            if self.0.borrow().err.is_some() {
                Err(self.0.borrow_mut().err.take().unwrap())
            } else {
                Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
            }
        } else {
            let size = std::cmp::min(self.0.borrow().read_buf.len(), dst.len());
            let b = self.0.borrow_mut().read_buf.split_to(size);
            dst[..size].copy_from_slice(&b);
            Ok(size)
        }
    }
}

impl io::Write for TestSeqBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write_buf.extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncRead for TestSeqBuffer {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let dst = buf.initialize_unfilled();
        let r = self.get_mut().read(dst);
        match r {
            Ok(n) => {
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

impl AsyncWrite for TestSeqBuffer {
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

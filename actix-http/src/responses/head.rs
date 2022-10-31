//! Response head type and caching pool.

use std::{cell::RefCell, ops};

use crate::{header::HeaderMap, message::Flags, ConnectionType, StatusCode, Version};

thread_local! {
    static RESPONSE_POOL: BoxedResponsePool = BoxedResponsePool::create();
}

#[derive(Debug, Clone)]
pub struct ResponseHead {
    pub version: Version,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub reason: Option<&'static str>,
    pub(crate) flags: Flags,
}

impl ResponseHead {
    /// Create new instance of `ResponseHead` type
    #[inline]
    pub fn new(status: StatusCode) -> ResponseHead {
        ResponseHead {
            status,
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(12),
            reason: None,
            flags: Flags::empty(),
        }
    }

    /// Read the message headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Mutable reference to the message headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Sets the flag that controls whether to send headers formatted as Camel-Case.
    ///
    /// Only applicable to HTTP/1.x responses; HTTP/2 header names are always lowercase.
    #[inline]
    pub fn set_camel_case_headers(&mut self, camel_case: bool) {
        if camel_case {
            self.flags.insert(Flags::CAMEL_CASE);
        } else {
            self.flags.remove(Flags::CAMEL_CASE);
        }
    }

    /// Set connection type of the message
    #[inline]
    pub fn set_connection_type(&mut self, ctype: ConnectionType) {
        match ctype {
            ConnectionType::Close => self.flags.insert(Flags::CLOSE),
            ConnectionType::KeepAlive => self.flags.insert(Flags::KEEP_ALIVE),
            ConnectionType::Upgrade => self.flags.insert(Flags::UPGRADE),
        }
    }

    #[inline]
    pub fn connection_type(&self) -> ConnectionType {
        if self.flags.contains(Flags::CLOSE) {
            ConnectionType::Close
        } else if self.flags.contains(Flags::KEEP_ALIVE) {
            ConnectionType::KeepAlive
        } else if self.flags.contains(Flags::UPGRADE) {
            ConnectionType::Upgrade
        } else if self.version < Version::HTTP_11 {
            ConnectionType::Close
        } else {
            ConnectionType::KeepAlive
        }
    }

    /// Check if keep-alive is enabled
    #[inline]
    pub fn keep_alive(&self) -> bool {
        self.connection_type() == ConnectionType::KeepAlive
    }

    /// Check upgrade status of this message
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.connection_type() == ConnectionType::Upgrade
    }

    /// Get custom reason for the response
    #[inline]
    pub fn reason(&self) -> &str {
        self.reason.unwrap_or_else(|| {
            self.status
                .canonical_reason()
                .unwrap_or("<unknown status code>")
        })
    }

    #[inline]
    pub(crate) fn conn_type(&self) -> Option<ConnectionType> {
        if self.flags.contains(Flags::CLOSE) {
            Some(ConnectionType::Close)
        } else if self.flags.contains(Flags::KEEP_ALIVE) {
            Some(ConnectionType::KeepAlive)
        } else if self.flags.contains(Flags::UPGRADE) {
            Some(ConnectionType::Upgrade)
        } else {
            None
        }
    }

    /// Get response body chunking state
    #[inline]
    pub fn chunked(&self) -> bool {
        !self.flags.contains(Flags::NO_CHUNKING)
    }

    /// Set no chunking for payload
    #[inline]
    pub fn no_chunking(&mut self, val: bool) {
        if val {
            self.flags.insert(Flags::NO_CHUNKING);
        } else {
            self.flags.remove(Flags::NO_CHUNKING);
        }
    }
}

pub(crate) struct BoxedResponseHead {
    head: Option<Box<ResponseHead>>,
}

impl BoxedResponseHead {
    /// Get new message from the pool of objects
    pub fn new(status: StatusCode) -> Self {
        RESPONSE_POOL.with(|p| p.get_message(status))
    }
}

impl ops::Deref for BoxedResponseHead {
    type Target = ResponseHead;

    fn deref(&self) -> &Self::Target {
        self.head.as_ref().unwrap()
    }
}

impl ops::DerefMut for BoxedResponseHead {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.head.as_mut().unwrap()
    }
}

impl Drop for BoxedResponseHead {
    fn drop(&mut self) {
        if let Some(head) = self.head.take() {
            RESPONSE_POOL.with(move |p| p.release(head))
        }
    }
}

/// Response head object pool.
#[doc(hidden)]
pub struct BoxedResponsePool(#[allow(clippy::vec_box)] RefCell<Vec<Box<ResponseHead>>>);

impl BoxedResponsePool {
    fn create() -> BoxedResponsePool {
        BoxedResponsePool(RefCell::new(Vec::with_capacity(128)))
    }

    /// Get message from the pool.
    #[inline]
    fn get_message(&self, status: StatusCode) -> BoxedResponseHead {
        if let Some(mut head) = self.0.borrow_mut().pop() {
            head.reason = None;
            head.status = status;
            head.headers.clear();
            head.flags = Flags::empty();
            BoxedResponseHead { head: Some(head) }
        } else {
            BoxedResponseHead {
                head: Some(Box::new(ResponseHead::new(status))),
            }
        }
    }

    /// Release request instance.
    #[inline]
    fn release(&self, msg: Box<ResponseHead>) {
        let pool = &mut self.0.borrow_mut();

        if pool.len() < 128 {
            pool.push(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        net,
    };

    use memchr::memmem;

    use crate::{
        h1::H1Service,
        header::{HeaderName, HeaderValue},
        Error, Request, Response, ServiceConfig,
    };

    #[actix_rt::test]
    async fn camel_case_headers() {
        let mut srv = actix_http_test::test_server(|| {
            H1Service::with_config(ServiceConfig::default(), |req: Request| async move {
                let mut res = Response::ok();

                if req.path().contains("camel") {
                    res.head_mut().set_camel_case_headers(true);
                }

                res.headers_mut().insert(
                    HeaderName::from_static("foo-bar"),
                    HeaderValue::from_static("baz"),
                );

                Ok::<_, Error>(res)
            })
            .tcp()
        })
        .await;

        let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
        stream
            .write_all(b"GET /camel HTTP/1.1\r\nConnection: Close\r\n\r\n")
            .unwrap();
        let mut data = vec![];
        let _ = stream.read_to_end(&mut data).unwrap();
        assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");
        assert!(memmem::find(&data, b"Foo-Bar").is_some());
        assert!(memmem::find(&data, b"foo-bar").is_none());
        assert!(memmem::find(&data, b"Date").is_some());
        assert!(memmem::find(&data, b"date").is_none());
        assert!(memmem::find(&data, b"Content-Length").is_some());
        assert!(memmem::find(&data, b"content-length").is_none());

        let mut stream = net::TcpStream::connect(srv.addr()).unwrap();
        stream
            .write_all(b"GET /lower HTTP/1.1\r\nConnection: Close\r\n\r\n")
            .unwrap();
        let mut data = vec![];
        let _ = stream.read_to_end(&mut data).unwrap();
        assert_eq!(&data[..17], b"HTTP/1.1 200 OK\r\n");
        assert!(memmem::find(&data, b"Foo-Bar").is_none());
        assert!(memmem::find(&data, b"foo-bar").is_some());
        assert!(memmem::find(&data, b"Date").is_none());
        assert!(memmem::find(&data, b"date").is_some());
        assert!(memmem::find(&data, b"Content-Length").is_none());
        assert!(memmem::find(&data, b"content-length").is_some());

        srv.stop().await;
    }
}

use std::cell::{Ref, RefMut};
use std::fmt;
use std::rc::Rc;

use http::{header, HeaderMap, Method, Uri, Version};

use extensions::Extensions;
use httpmessage::HttpMessage;
use payload::Payload;

use message::{Head, Message, MessagePool, RequestHead};

/// Request
pub struct Request {
    pub(crate) inner: Rc<Message<RequestHead>>,
}

impl HttpMessage for Request {
    type Stream = Payload;

    fn headers(&self) -> &HeaderMap {
        &self.inner.head.headers
    }

    #[inline]
    fn payload(&self) -> Payload {
        if let Some(payload) = self.inner.payload.borrow_mut().take() {
            payload
        } else {
            Payload::empty()
        }
    }
}

impl Request {
    /// Create new Request instance
    pub fn new() -> Request {
        Request {
            inner: MessagePool::get_message(),
        }
    }

    // /// Create new Request instance with pool
    // pub(crate) fn with_pool(pool: &'static MessagePool) -> Request {
    //     Request {
    //         inner: Rc::new(Message {
    //             pool,
    //             url: Url::default(),
    //             head: RequestHead::default(),
    //             status: StatusCode::OK,
    //             flags: Cell::new(MessageFlags::empty()),
    //             payload: RefCell::new(None),
    //             extensions: RefCell::new(Extensions::new()),
    //         }),
    //     }
    // }

    #[inline]
    #[doc(hidden)]
    pub fn inner(&self) -> &Message<RequestHead> {
        self.inner.as_ref()
    }

    #[inline]
    #[doc(hidden)]
    pub fn inner_mut(&mut self) -> &mut Message<RequestHead> {
        Rc::get_mut(&mut self.inner).expect("Multiple copies exist")
    }

    /// Request's uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.inner().head.uri
    }

    /// Mutable reference to the request's uri.
    #[inline]
    pub fn uri_mut(&mut self) -> &mut Uri {
        &mut self.inner_mut().head.uri
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.inner().head.method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.inner().head.version
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.inner().head.uri.path()
    }

    #[inline]
    /// Returns Request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.inner().head.headers
    }

    #[inline]
    /// Returns mutable Request's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.inner_mut().head.headers
    }

    /// Checks if a connection should be kept alive.
    #[inline]
    pub fn keep_alive(&self) -> bool {
        self.inner().head.keep_alive()
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.inner().extensions.borrow()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.inner().extensions.borrow_mut()
    }

    /// Check if request requires connection upgrade
    pub fn upgrade(&self) -> bool {
        if let Some(conn) = self.inner().head.headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade");
            }
        }
        self.inner().head.method == Method::CONNECT
    }

    #[doc(hidden)]
    /// Note: this method should be called only as part of clone operation
    /// of wrapper type.
    pub fn clone_request(&self) -> Self {
        Request {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) == 1 {
            self.inner.pool.release(self.inner.clone());
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "\nRequest {:?} {}:{}",
            self.version(),
            self.method(),
            self.path()
        )?;
        if let Some(q) = self.uri().query().as_ref() {
            writeln!(f, "  query: ?{:?}", q)?;
        }
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

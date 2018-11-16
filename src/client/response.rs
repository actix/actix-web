use std::cell::{Cell, Ref, RefCell, RefMut};
use std::fmt;
use std::rc::Rc;

use bytes::Bytes;
use futures::{Async, Poll, Stream};
use http::{HeaderMap, StatusCode, Version};

use body::PayloadStream;
use error::PayloadError;
use extensions::Extensions;
use httpmessage::HttpMessage;
use request::{Message, MessageFlags, MessagePool, RequestHead};
use uri::Url;

use super::pipeline::Payload;

/// Client Response
pub struct ClientResponse {
    pub(crate) inner: Rc<Message>,
    pub(crate) payload: RefCell<Option<PayloadStream>>,
}

impl HttpMessage for ClientResponse {
    type Stream = PayloadStream;

    fn headers(&self) -> &HeaderMap {
        &self.inner.head.headers
    }

    #[inline]
    fn payload(&self) -> Self::Stream {
        if let Some(payload) = self.payload.borrow_mut().take() {
            payload
        } else {
            Payload::empty()
        }
    }
}

impl ClientResponse {
    /// Create new Request instance
    pub fn new() -> ClientResponse {
        ClientResponse::with_pool(MessagePool::pool())
    }

    /// Create new Request instance with pool
    pub(crate) fn with_pool(pool: &'static MessagePool) -> ClientResponse {
        ClientResponse {
            inner: Rc::new(Message {
                pool,
                head: RequestHead::default(),
                status: StatusCode::OK,
                url: Url::default(),
                flags: Cell::new(MessageFlags::empty()),
                payload: RefCell::new(None),
                extensions: RefCell::new(Extensions::new()),
            }),
            payload: RefCell::new(None),
        }
    }

    #[inline]
    pub(crate) fn inner(&self) -> &Message {
        self.inner.as_ref()
    }

    #[inline]
    pub(crate) fn inner_mut(&mut self) -> &mut Message {
        Rc::get_mut(&mut self.inner).expect("Multiple copies exist")
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.inner().head.version
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.inner().status
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
        self.inner().flags.get().contains(MessageFlags::KEEPALIVE)
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
}

impl Stream for ClientResponse {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(ref mut payload) = &mut *self.payload.borrow_mut() {
            payload.poll()
        } else {
            Ok(Async::Ready(None))
        }
    }
}

impl Drop for ClientResponse {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) == 1 {
            self.inner.pool.release(self.inner.clone());
        }
    }
}

impl fmt::Debug for ClientResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "\nClientResponse {:?} {}", self.version(), self.status(),)?;
        writeln!(f, "  headers:")?;
        for (key, val) in self.headers().iter() {
            writeln!(f, "    {:?}: {:?}", key, val)?;
        }
        Ok(())
    }
}

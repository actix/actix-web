use std::cell::{Ref, RefMut};
use std::fmt;
use std::rc::Rc;

use bytes::Bytes;
use futures::Stream;
use http::{header, HeaderMap, Method, Uri, Version};

use crate::error::PayloadError;
use crate::extensions::Extensions;
use crate::httpmessage::HttpMessage;
use crate::message::{Message, MessagePool, RequestHead};
use crate::payload::Payload;

/// Request
pub struct Request<P = Payload> {
    pub(crate) payload: Option<P>,
    pub(crate) inner: Rc<Message<RequestHead>>,
}

impl<P> HttpMessage for Request<P>
where
    P: Stream<Item = Bytes, Error = PayloadError>,
{
    type Stream = P;

    fn headers(&self) -> &HeaderMap {
        &self.inner.head.headers
    }

    #[inline]
    fn payload(&mut self) -> Option<P> {
        self.payload.take()
    }
}

impl Request<Payload> {
    /// Create new Request instance
    pub fn new() -> Request<Payload> {
        Request {
            payload: None,
            inner: MessagePool::get_message(),
        }
    }
}

impl<Payload> Request<Payload> {
    /// Create new Request instance
    pub fn with_payload(payload: Payload) -> Request<Payload> {
        Request {
            payload: Some(payload),
            inner: MessagePool::get_message(),
        }
    }

    /// Create new Request instance
    pub fn set_payload<I, P>(self, payload: I) -> Request<P>
    where
        I: Into<P>,
    {
        Request {
            payload: Some(payload.into()),
            inner: self.inner.clone(),
        }
    }

    /// Take request's payload
    pub fn take_payload(mut self) -> (Option<Payload>, Request<()>) {
        (
            self.payload.take(),
            Request {
                payload: Some(()),
                inner: self.inner.clone(),
            },
        )
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

    #[inline]
    /// Http message part of the request
    pub fn head(&self) -> &RequestHead {
        &self.inner.as_ref().head
    }

    #[inline]
    #[doc(hidden)]
    /// Mutable reference to a http message part of the request
    pub fn head_mut(&mut self) -> &mut RequestHead {
        &mut self.inner_mut().head
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

    // #[doc(hidden)]
    // /// Note: this method should be called only as part of clone operation
    // /// of wrapper type.
    // pub fn clone_request(&self) -> Self {
    //     Request {
    //         inner: self.inner.clone(),
    //     }
    // }
}

impl<Payload> Drop for Request<Payload> {
    fn drop(&mut self) {
        if Rc::strong_count(&self.inner) == 1 {
            self.inner.pool.release(self.inner.clone());
        }
    }
}

impl<Payload> fmt::Debug for Request<Payload> {
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

use std::cell::{Ref, RefCell, RefMut};
use std::{fmt, mem};

use bytes::Bytes;
use futures::Stream;
use http::{header, HeaderMap, Method, Uri, Version};

use crate::error::PayloadError;
use crate::extensions::Extensions;
use crate::httpmessage::HttpMessage;
use crate::message::{Message, RequestHead};
use crate::payload::{Payload, PayloadStream};

/// Request
pub struct Request<P = PayloadStream> {
    pub(crate) payload: RefCell<Payload<P>>,
    pub(crate) inner: Message<RequestHead>,
}

impl<P> HttpMessage for Request<P>
where
    P: Stream<Item = Bytes, Error = PayloadError>,
{
    type Stream = P;

    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    #[inline]
    fn payload(&self) -> Payload<Self::Stream> {
        mem::replace(&mut *self.payload.borrow_mut(), Payload::None)
    }
}

impl<P> From<Message<RequestHead>> for Request<P> {
    fn from(msg: Message<RequestHead>) -> Self {
        Request {
            payload: RefCell::new(Payload::None),
            inner: msg,
        }
    }
}

impl Request<PayloadStream> {
    /// Create new Request instance
    pub fn new() -> Request<PayloadStream> {
        Request {
            payload: RefCell::new(Payload::None),
            inner: Message::new(),
        }
    }
}

impl<P> Request<P> {
    /// Create new Request instance
    pub fn with_payload(payload: Payload<P>) -> Request<P> {
        Request {
            payload: RefCell::new(payload),
            inner: Message::new(),
        }
    }

    /// Create new Request instance
    pub fn set_payload<I, P1>(self, payload: I) -> Request<P1>
    where
        I: Into<Payload<P1>>,
    {
        Request {
            payload: RefCell::new(payload.into()),
            inner: self.inner,
        }
    }

    /// Split request into request head and payload
    pub fn into_parts(self) -> (Message<RequestHead>, Payload<P>) {
        (self.inner, self.payload.into_inner())
    }

    #[inline]
    /// Http message part of the request
    pub fn head(&self) -> &RequestHead {
        &*self.inner
    }

    #[inline]
    #[doc(hidden)]
    /// Mutable reference to a http message part of the request
    pub fn head_mut(&mut self) -> &mut RequestHead {
        &mut *self.inner
    }

    /// Request's uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.head().uri
    }

    /// Mutable reference to the request's uri.
    #[inline]
    pub fn uri_mut(&mut self) -> &mut Uri {
        &mut self.head_mut().uri
    }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.head().method
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    /// The target path of this Request.
    #[inline]
    pub fn path(&self) -> &str {
        self.head().uri.path()
    }

    #[inline]
    /// Returns Request's headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    #[inline]
    /// Returns mutable Request's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head_mut().headers
    }

    /// Request extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.inner.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.inner.extensions_mut()
    }

    /// Check if request requires connection upgrade
    pub fn upgrade(&self) -> bool {
        if let Some(conn) = self.head().headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade");
            }
        }
        self.head().method == Method::CONNECT
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

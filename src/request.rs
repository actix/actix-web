use std::cell::{Ref, RefMut};
use std::fmt;

use http::{header, HeaderMap, Method, Uri, Version};

use crate::extensions::Extensions;
use crate::httpmessage::HttpMessage;
use crate::message::{Message, RequestHead};
use crate::payload::{Payload, PayloadStream};

/// Request
pub struct Request<P = PayloadStream> {
    pub(crate) payload: Payload<P>,
    pub(crate) head: Message<RequestHead>,
}

impl<P> HttpMessage for Request<P> {
    type Stream = P;

    #[inline]
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    /// Request extensions
    #[inline]
    fn extensions(&self) -> Ref<Extensions> {
        self.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.head.extensions_mut()
    }

    fn take_payload(&mut self) -> Payload<P> {
        std::mem::replace(&mut self.payload, Payload::None)
    }
}

impl From<Message<RequestHead>> for Request<PayloadStream> {
    fn from(head: Message<RequestHead>) -> Self {
        Request {
            head,
            payload: Payload::None,
        }
    }
}

impl Request<PayloadStream> {
    /// Create new Request instance
    pub fn new() -> Request<PayloadStream> {
        Request {
            head: Message::new(),
            payload: Payload::None,
        }
    }
}

impl<P> Request<P> {
    /// Create new Request instance
    pub fn with_payload(payload: Payload<P>) -> Request<P> {
        Request {
            payload,
            head: Message::new(),
        }
    }

    /// Create new Request instance
    pub fn replace_payload<P1>(self, payload: Payload<P1>) -> (Request<P1>, Payload<P>) {
        let pl = self.payload;
        (
            Request {
                payload,
                head: self.head,
            },
            pl,
        )
    }

    /// Get request's payload
    pub fn take_payload(&mut self) -> Payload<P> {
        std::mem::replace(&mut self.payload, Payload::None)
    }

    /// Split request into request head and payload
    pub fn into_parts(self) -> (Message<RequestHead>, Payload<P>) {
        (self.head, self.payload)
    }

    #[inline]
    /// Http message part of the request
    pub fn head(&self) -> &RequestHead {
        &*self.head
    }

    #[inline]
    #[doc(hidden)]
    /// Mutable reference to a http message part of the request
    pub fn head_mut(&mut self) -> &mut RequestHead {
        &mut *self.head
    }

    /// Mutable reference to the message's headers.
    fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head_mut().headers
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

impl<P> fmt::Debug for Request<P> {
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

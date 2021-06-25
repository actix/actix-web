//! HTTP requests.

use std::{
    cell::{Ref, RefMut},
    fmt, net, str,
};

use http::{header, Method, Uri, Version};

use crate::{
    extensions::Extensions,
    header::HeaderMap,
    message::{Message, RequestHead},
    payload::{Payload, PayloadStream},
    HttpMessage,
};

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

    fn take_payload(&mut self) -> Payload<P> {
        std::mem::replace(&mut self.payload, Payload::None)
    }

    /// Request extensions
    #[inline]
    fn extensions(&self) -> Ref<'_, Extensions> {
        self.head.extensions()
    }

    /// Mutable reference to a the request's extensions
    #[inline]
    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.head.extensions_mut()
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
    pub fn payload(&mut self) -> &mut Payload<P> {
        &mut self.payload
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
    /// Mutable reference to a HTTP message part of the request
    pub fn head_mut(&mut self) -> &mut RequestHead {
        &mut *self.head
    }

    /// Mutable reference to the message's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
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
    #[inline]
    pub fn upgrade(&self) -> bool {
        if let Some(conn) = self.head().headers.get(header::CONNECTION) {
            if let Ok(s) = conn.to_str() {
                return s.to_lowercase().contains("upgrade");
            }
        }
        self.head().method == Method::CONNECT
    }

    /// Peer socket address.
    ///
    /// Peer address is the directly connected peer's socket address. If a proxy is used in front of
    /// the Actix Web server, then it would be address of this proxy.
    ///
    /// Will only return None when called in unit tests.
    #[inline]
    pub fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.head().peer_addr
    }
}

impl<P> fmt::Debug for Request<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn test_basics() {
        let msg = Message::new();
        let mut req = Request::from(msg);
        req.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain"),
        );
        assert!(req.headers().contains_key(header::CONTENT_TYPE));

        *req.uri_mut() = Uri::try_from("/index.html?q=1").unwrap();
        assert_eq!(req.uri().path(), "/index.html");
        assert_eq!(req.uri().query(), Some("q=1"));

        let s = format!("{:?}", req);
        assert!(s.contains("Request HTTP/1.1 GET:/index.html"));
    }
}

//! HTTP requests.

use std::{
    cell::{Ref, RefCell, RefMut},
    fmt, mem, net,
    rc::Rc,
    str,
};

use http::{header, Method, Uri, Version};

use crate::{
    header::HeaderMap, BoxedPayloadStream, Extensions, HttpMessage, Message, Payload, RequestHead,
};

/// An HTTP request.
pub struct Request<P = BoxedPayloadStream> {
    pub(crate) payload: Payload<P>,
    pub(crate) head: Message<RequestHead>,
    pub(crate) conn_data: Option<Rc<Extensions>>,
    pub(crate) extensions: RefCell<Extensions>,
}

impl<P> HttpMessage for Request<P> {
    type Stream = P;

    #[inline]
    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    fn take_payload(&mut self) -> Payload<P> {
        mem::replace(&mut self.payload, Payload::None)
    }

    #[inline]
    fn extensions(&self) -> Ref<'_, Extensions> {
        self.extensions.borrow()
    }

    #[inline]
    fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions.borrow_mut()
    }
}

impl From<Message<RequestHead>> for Request<BoxedPayloadStream> {
    fn from(head: Message<RequestHead>) -> Self {
        Request {
            head,
            payload: Payload::None,
            extensions: RefCell::new(Extensions::default()),
            conn_data: None,
        }
    }
}

impl Request<BoxedPayloadStream> {
    /// Create new Request instance
    #[allow(clippy::new_without_default)]
    pub fn new() -> Request<BoxedPayloadStream> {
        Request {
            head: Message::new(),
            payload: Payload::None,
            extensions: RefCell::new(Extensions::default()),
            conn_data: None,
        }
    }
}

impl<P> Request<P> {
    /// Create new Request instance
    pub fn with_payload(payload: Payload<P>) -> Request<P> {
        Request {
            payload,
            head: Message::new(),
            extensions: RefCell::new(Extensions::default()),
            conn_data: None,
        }
    }

    /// Create new Request instance
    pub fn replace_payload<P1>(self, payload: Payload<P1>) -> (Request<P1>, Payload<P>) {
        let pl = self.payload;

        (
            Request {
                payload,
                head: self.head,
                extensions: self.extensions,
                conn_data: self.conn_data,
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
        mem::replace(&mut self.payload, Payload::None)
    }

    /// Split request into request head and payload
    pub fn into_parts(self) -> (Message<RequestHead>, Payload<P>) {
        (self.head, self.payload)
    }

    #[inline]
    /// Http message part of the request
    pub fn head(&self) -> &RequestHead {
        &self.head
    }

    #[inline]
    #[doc(hidden)]
    /// Mutable reference to a HTTP message part of the request
    pub fn head_mut(&mut self) -> &mut RequestHead {
        &mut self.head
    }

    /// Mutable reference to the message's headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.head.headers
    }

    /// Request's uri.
    #[inline]
    pub fn uri(&self) -> &Uri {
        &self.head().uri
    }

    /// Mutable reference to the request's uri.
    #[inline]
    pub fn uri_mut(&mut self) -> &mut Uri {
        &mut self.head.uri
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
    /// Will only return None when called in unit tests unless set manually.
    #[inline]
    pub fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.head().peer_addr
    }

    /// Returns a reference a piece of connection data set in an [on-connect] callback.
    ///
    /// ```ignore
    /// let opt_t = req.conn_data::<PeerCertificate>();
    /// ```
    ///
    /// [on-connect]: crate::HttpServiceBuilder::on_connect_ext
    pub fn conn_data<T: 'static>(&self) -> Option<&T> {
        self.conn_data
            .as_deref()
            .and_then(|container| container.get::<T>())
    }

    /// Returns the connection-level data/extensions container if an [on-connect] callback was
    /// registered, leaving an empty one in its place.
    ///
    /// [on-connect]: crate::HttpServiceBuilder::on_connect_ext
    pub fn take_conn_data(&mut self) -> Option<Rc<Extensions>> {
        self.conn_data.take()
    }

    /// Returns the request-local data/extensions container, leaving an empty one in its place.
    pub fn take_req_data(&mut self) -> Extensions {
        mem::take(self.extensions.get_mut())
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

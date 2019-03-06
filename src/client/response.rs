use std::cell::{Ref, RefMut};
use std::fmt;

use bytes::Bytes;
use futures::{Poll, Stream};
use http::{HeaderMap, StatusCode, Version};

use crate::error::PayloadError;
use crate::extensions::Extensions;
use crate::httpmessage::HttpMessage;
use crate::message::{Head, Message, ResponseHead};
use crate::payload::{Payload, PayloadStream};

/// Client Response
pub struct ClientResponse {
    pub(crate) head: Message<ResponseHead>,
    pub(crate) payload: Payload,
}

impl HttpMessage for ClientResponse {
    type Stream = PayloadStream;

    fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    fn extensions(&self) -> Ref<Extensions> {
        self.head.extensions()
    }

    fn extensions_mut(&self) -> RefMut<Extensions> {
        self.head.extensions_mut()
    }

    fn take_payload(&mut self) -> Payload {
        std::mem::replace(&mut self.payload, Payload::None)
    }
}

impl ClientResponse {
    /// Create new Request instance
    pub fn new() -> ClientResponse {
        let head: Message<ResponseHead> = Message::new();
        head.extensions_mut().clear();

        ClientResponse {
            head,
            payload: Payload::None,
        }
    }

    #[inline]
    pub(crate) fn head(&self) -> &ResponseHead {
        &self.head
    }

    #[inline]
    pub(crate) fn head_mut(&mut self) -> &mut ResponseHead {
        &mut self.head
    }

    /// Read the Request Version.
    #[inline]
    pub fn version(&self) -> Version {
        self.head().version
    }

    /// Get the status from the server.
    #[inline]
    pub fn status(&self) -> StatusCode {
        self.head().status
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

    /// Checks if a connection should be kept alive.
    #[inline]
    pub fn keep_alive(&self) -> bool {
        self.head().keep_alive()
    }

    /// Set response payload
    pub fn set_payload(&mut self, payload: Payload) {
        self.payload = payload;
    }
}

impl Stream for ClientResponse {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.payload.poll()
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

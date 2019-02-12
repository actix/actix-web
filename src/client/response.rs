use std::cell::RefCell;
use std::{fmt, mem};

use bytes::Bytes;
use futures::{Poll, Stream};
use http::{HeaderMap, StatusCode, Version};

use crate::error::PayloadError;
use crate::httpmessage::HttpMessage;
use crate::message::{Head, ResponseHead};
use crate::payload::{Payload, PayloadStream};

/// Client Response
pub struct ClientResponse {
    pub(crate) head: ResponseHead,
    pub(crate) payload: RefCell<Payload>,
}

impl HttpMessage for ClientResponse {
    type Stream = PayloadStream;

    fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    #[inline]
    fn payload(&self) -> Payload<Self::Stream> {
        mem::replace(&mut *self.payload.borrow_mut(), Payload::None)
    }
}

impl ClientResponse {
    /// Create new Request instance
    pub fn new() -> ClientResponse {
        ClientResponse {
            head: ResponseHead::default(),
            payload: RefCell::new(Payload::None),
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
        *self.payload.get_mut() = payload;
    }
}

impl Stream for ClientResponse {
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.payload.get_mut().poll()
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

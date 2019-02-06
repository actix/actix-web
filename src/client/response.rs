use std::cell::RefCell;
use std::fmt;

use bytes::Bytes;
use futures::{Async, Poll, Stream};
use http::{HeaderMap, StatusCode, Version};

use crate::body::PayloadStream;
use crate::error::PayloadError;
use crate::httpmessage::HttpMessage;
use crate::message::{Head, ResponseHead};

use super::h1proto::Payload;

/// Client Response
#[derive(Default)]
pub struct ClientResponse {
    pub(crate) head: ResponseHead,
    pub(crate) payload: RefCell<Option<PayloadStream>>,
}

impl HttpMessage for ClientResponse {
    type Stream = PayloadStream;

    fn headers(&self) -> &HeaderMap {
        &self.head.headers
    }

    #[inline]
    fn payload(self) -> Self::Stream {
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
        ClientResponse {
            head: ResponseHead::default(),
            payload: RefCell::new(None),
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

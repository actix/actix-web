use std::{
    cell::{RefCell, RefMut},
    cmp, mem,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_web::{
    error::PayloadError,
    web::{Bytes, BytesMut},
};
use futures_core::stream::{LocalBoxStream, Stream};

use crate::{error::Error, safety::Safety};

pub(crate) struct PayloadRef {
    payload: Rc<RefCell<PayloadBuffer>>,
}

impl PayloadRef {
    pub(crate) fn new(payload: PayloadBuffer) -> PayloadRef {
        PayloadRef {
            payload: Rc::new(RefCell::new(payload)),
        }
    }

    pub(crate) fn get_mut(&self, safety: &Safety) -> Option<RefMut<'_, PayloadBuffer>> {
        if safety.current() {
            Some(self.payload.borrow_mut())
        } else {
            None
        }
    }
}

impl Clone for PayloadRef {
    fn clone(&self) -> PayloadRef {
        PayloadRef {
            payload: Rc::clone(&self.payload),
        }
    }
}

/// Payload buffer.
pub(crate) struct PayloadBuffer {
    pub(crate) stream: LocalBoxStream<'static, Result<Bytes, PayloadError>>,
    pub(crate) buf: BytesMut,
    /// EOF flag. If true, no more payload reads will be attempted.
    pub(crate) eof: bool,
}

impl PayloadBuffer {
    /// Constructs new payload buffer.
    pub(crate) fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        PayloadBuffer {
            stream: Box::pin(stream),
            buf: BytesMut::with_capacity(1_024), // pre-allocate 1KiB
            eof: false,
        }
    }

    pub(crate) fn poll_stream(&mut self, cx: &mut Context<'_>) -> Result<(), PayloadError> {
        loop {
            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => {
                    self.buf.extend_from_slice(&data);
                    // try to read more data
                    continue;
                }
                Poll::Ready(Some(Err(err))) => return Err(err),
                Poll::Ready(None) => {
                    self.eof = true;
                    return Ok(());
                }
                Poll::Pending => return Ok(()),
            }
        }
    }

    /// Reads exact number of bytes.
    #[cfg(test)]
    pub(crate) fn read_exact(&mut self, size: usize) -> Option<Bytes> {
        if size <= self.buf.len() {
            Some(self.buf.split_to(size).freeze())
        } else {
            None
        }
    }

    pub(crate) fn read_max(&mut self, size: u64) -> Result<Option<Bytes>, Error> {
        if !self.buf.is_empty() {
            let size = cmp::min(self.buf.len() as u64, size) as usize;
            Ok(Some(self.buf.split_to(size).freeze()))
        } else if self.eof {
            Err(Error::Incomplete)
        } else {
            Ok(None)
        }
    }

    /// Reads until specified ending.
    ///
    /// Returns:
    ///
    /// - `Ok(Some(chunk))` - `needle` is found, with chunk ending after needle
    /// - `Err(Incomplete)` - `needle` is not found and we're at EOF
    /// - `Ok(None)` - `needle` is not found otherwise
    pub(crate) fn read_until(&mut self, needle: &[u8]) -> Result<Option<Bytes>, Error> {
        match memchr::memmem::find(&self.buf, needle) {
            // buffer exhausted and EOF without finding needle
            None if self.eof => Err(Error::Incomplete),

            // needle not yet found
            None => Ok(None),

            // needle found, split chunk out of buf
            Some(idx) => Ok(Some(self.buf.split_to(idx + needle.len()).freeze())),
        }
    }

    /// Reads bytes until new line delimiter (`\n`, `0x0A`).
    ///
    /// Returns:
    ///
    /// - `Ok(Some(chunk))` - `needle` is found, with chunk ending after needle
    /// - `Err(Incomplete)` - `needle` is not found and we're at EOF
    /// - `Ok(None)` - `needle` is not found otherwise
    #[inline]
    pub(crate) fn readline(&mut self) -> Result<Option<Bytes>, Error> {
        self.read_until(b"\n")
    }

    /// Reads bytes until new line delimiter or until EOF.
    #[inline]
    pub(crate) fn readline_or_eof(&mut self) -> Result<Option<Bytes>, Error> {
        match self.readline() {
            Err(Error::Incomplete) if self.eof => Ok(Some(self.buf.split().freeze())),
            line => line,
        }
    }

    /// Puts unprocessed data back to the buffer.
    pub(crate) fn unprocessed(&mut self, data: Bytes) {
        // TODO: use BytesMut::from when it's released, see https://github.com/tokio-rs/bytes/pull/710
        let buf = BytesMut::from(&data[..]);
        let buf = mem::replace(&mut self.buf, buf);
        self.buf.extend_from_slice(&buf);
    }
}

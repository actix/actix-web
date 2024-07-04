use std::{
    cell::{RefCell, RefMut},
    cmp,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_web::{
    error::PayloadError,
    web::{Bytes, BytesMut},
};
use futures_core::stream::{LocalBoxStream, Stream};

use crate::{error::MultipartError, safety::Safety};

pub(crate) struct PayloadRef {
    payload: Rc<RefCell<PayloadBuffer>>,
}

impl PayloadRef {
    pub(crate) fn new(payload: PayloadBuffer) -> PayloadRef {
        PayloadRef {
            payload: Rc::new(payload.into()),
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
    pub(crate) eof: bool,
    pub(crate) buf: BytesMut,
    pub(crate) stream: LocalBoxStream<'static, Result<Bytes, PayloadError>>,
}

impl PayloadBuffer {
    /// Constructs new `PayloadBuffer` instance.
    pub(crate) fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        PayloadBuffer {
            eof: false,
            buf: BytesMut::new(),
            stream: Box::pin(stream),
        }
    }

    pub(crate) fn poll_stream(&mut self, cx: &mut Context<'_>) -> Result<(), PayloadError> {
        loop {
            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => self.buf.extend_from_slice(&data),
                Poll::Ready(Some(Err(err))) => return Err(err),
                Poll::Ready(None) => {
                    self.eof = true;
                    return Ok(());
                }
                Poll::Pending => return Ok(()),
            }
        }
    }

    /// Read exact number of bytes.
    #[cfg(test)]
    pub(crate) fn read_exact(&mut self, size: usize) -> Option<Bytes> {
        if size <= self.buf.len() {
            Some(self.buf.split_to(size).freeze())
        } else {
            None
        }
    }

    pub(crate) fn read_max(&mut self, size: u64) -> Result<Option<Bytes>, MultipartError> {
        if !self.buf.is_empty() {
            let size = cmp::min(self.buf.len() as u64, size) as usize;
            Ok(Some(self.buf.split_to(size).freeze()))
        } else if self.eof {
            Err(MultipartError::Incomplete)
        } else {
            Ok(None)
        }
    }

    /// Read until specified ending.
    pub(crate) fn read_until(&mut self, line: &[u8]) -> Result<Option<Bytes>, MultipartError> {
        let res = memchr::memmem::find(&self.buf, line)
            .map(|idx| self.buf.split_to(idx + line.len()).freeze());

        if res.is_none() && self.eof {
            Err(MultipartError::Incomplete)
        } else {
            Ok(res)
        }
    }

    /// Read bytes until new line delimiter.
    pub(crate) fn readline(&mut self) -> Result<Option<Bytes>, MultipartError> {
        self.read_until(b"\n")
    }

    /// Read bytes until new line delimiter or EOF.
    pub(crate) fn readline_or_eof(&mut self) -> Result<Option<Bytes>, MultipartError> {
        match self.readline() {
            Err(MultipartError::Incomplete) if self.eof => Ok(Some(self.buf.split().freeze())),
            line => line,
        }
    }

    /// Put unprocessed data back to the buffer.
    pub(crate) fn unprocessed(&mut self, data: Bytes) {
        let buf = BytesMut::from(data.as_ref());
        let buf = std::mem::replace(&mut self.buf, buf);
        self.buf.extend_from_slice(&buf);
    }
}

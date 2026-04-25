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

pub(crate) const DEFAULT_BUFFER_LIMIT: usize = 65_536; // 64 KiB
const MAX_READY_CHUNKS_PER_POLL: usize = 16;

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
    pending: Option<Bytes>,
    pub(crate) buf: BytesMut,
    buffer_limit: usize,
    /// EOF flag. If true, no more payload reads will be attempted.
    pub(crate) eof: bool,
}

impl PayloadBuffer {
    /// Constructs new payload buffer.
    pub(crate) fn new_with_limit<S>(stream: S, buffer_limit: usize) -> Self
    where
        S: Stream<Item = Result<Bytes, PayloadError>> + 'static,
    {
        PayloadBuffer {
            stream: Box::pin(stream),
            pending: None,
            buf: BytesMut::with_capacity(1_024), // pre-allocate 1KiB
            buffer_limit,
            eof: false,
        }
    }

    /// Polls a bounded amount of payload into the parser buffer.
    ///
    /// This does not drain the stream to EOF in one call. Callers must be prepared to poll again
    /// after consuming buffered data.
    pub(crate) fn poll_stream(&mut self, cx: &mut Context<'_>) -> Result<(), PayloadError> {
        if self.buffer_limit == 0 {
            return Err(PayloadError::Overflow);
        }

        let mut appended = false;

        for _ in 0..MAX_READY_CHUNKS_PER_POLL {
            if self.pending.is_some() {
                appended |= self.append_pending()?;

                if self.pending.is_some() || self.buf.len() >= self.buffer_limit {
                    if appended {
                        cx.waker().wake_by_ref();
                    }
                    return Ok(());
                }

                continue;
            }

            match Pin::new(&mut self.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => {
                    self.pending = Some(data);
                    appended |= self.append_pending()?;

                    if self.pending.is_some() || self.buf.len() >= self.buffer_limit {
                        if appended {
                            cx.waker().wake_by_ref();
                        }
                        return Ok(());
                    }
                }
                Poll::Ready(Some(Err(err))) => return Err(err),
                Poll::Ready(None) => {
                    self.eof = true;
                    return Ok(());
                }
                Poll::Pending => return Ok(()),
            }
        }

        if appended {
            cx.waker().wake_by_ref();
        }

        Ok(())
    }

    fn append_pending(&mut self) -> Result<bool, PayloadError> {
        let Some(mut data) = self.pending.take() else {
            return Ok(false);
        };

        if data.is_empty() {
            return Ok(false);
        }

        if self.buf.len() >= self.buffer_limit {
            self.pending = Some(data);
            return Err(PayloadError::Overflow);
        }

        let available = self.buffer_limit - self.buf.len();
        let len = cmp::min(data.len(), available);

        if len == data.len() {
            self.buf.extend_from_slice(&data);
        } else {
            let chunk = data.split_to(len);
            self.buf.extend_from_slice(&chunk);
            self.pending = Some(data);
        }

        Ok(len != 0)
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

#[cfg(test)]
mod tests {
    use actix_http::h1;
    use futures_util::future::lazy;

    use super::*;

    #[actix_rt::test]
    async fn basic() {
        let (_sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new_with_limit(payload, DEFAULT_BUFFER_LIMIT);

        assert_eq!(payload.buf.len(), 0);
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(None, payload.read_max(1).unwrap());
    }

    #[actix_rt::test]
    async fn eof() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new_with_limit(payload, DEFAULT_BUFFER_LIMIT);

        assert_eq!(None, payload.read_max(4).unwrap());
        sender.feed_data(Bytes::from("data"));
        sender.feed_eof();
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();

        assert_eq!(Some(Bytes::from("data")), payload.read_max(4).unwrap());
        assert_eq!(payload.buf.len(), 0);

        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert!(payload.read_max(1).is_err());
        assert!(payload.eof);
    }

    #[actix_rt::test]
    async fn err() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new_with_limit(payload, DEFAULT_BUFFER_LIMIT);
        assert_eq!(None, payload.read_max(1).unwrap());
        sender.set_error(PayloadError::Incomplete(None));
        lazy(|cx| payload.poll_stream(cx)).await.err().unwrap();
    }

    #[actix_rt::test]
    async fn read_max() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new_with_limit(payload, DEFAULT_BUFFER_LIMIT);

        sender.feed_data(Bytes::from("line1"));
        sender.feed_data(Bytes::from("line2"));
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(payload.buf.len(), 10);

        assert_eq!(Some(Bytes::from("line1")), payload.read_max(5).unwrap());
        assert_eq!(payload.buf.len(), 5);

        assert_eq!(Some(Bytes::from("line2")), payload.read_max(5).unwrap());
        assert_eq!(payload.buf.len(), 0);
    }

    #[actix_rt::test]
    async fn read_exactly() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new_with_limit(payload, DEFAULT_BUFFER_LIMIT);

        assert_eq!(None, payload.read_exact(2));

        sender.feed_data(Bytes::from("line1"));
        sender.feed_data(Bytes::from("line2"));
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();

        assert_eq!(Some(Bytes::from_static(b"li")), payload.read_exact(2));
        assert_eq!(payload.buf.len(), 8);

        assert_eq!(Some(Bytes::from_static(b"ne1l")), payload.read_exact(4));
        assert_eq!(payload.buf.len(), 4);
    }

    #[actix_rt::test]
    async fn read_until() {
        let (mut sender, payload) = h1::Payload::create(false);
        let mut payload = PayloadBuffer::new_with_limit(payload, DEFAULT_BUFFER_LIMIT);

        assert_eq!(None, payload.read_until(b"ne").unwrap());

        sender.feed_data(Bytes::from("line1"));
        sender.feed_data(Bytes::from("line2"));
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        lazy(|cx| payload.poll_stream(cx)).await.unwrap();

        assert_eq!(
            Some(Bytes::from("line")),
            payload.read_until(b"ne").unwrap()
        );
        assert_eq!(payload.buf.len(), 6);

        assert_eq!(
            Some(Bytes::from("1line2")),
            payload.read_until(b"2").unwrap()
        );
        assert_eq!(payload.buf.len(), 0);
    }

    #[actix_rt::test]
    async fn poll_stream_does_not_exceed_buffer_limit() {
        let stream = futures_util::stream::iter([
            Ok(Bytes::from_static(b"12345678")),
            Ok(Bytes::from_static(b"abcdefgh")),
            Ok(Bytes::from_static(b"overflow")),
        ]);
        let mut payload = PayloadBuffer::new_with_limit(stream, 16);

        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(payload.buf.len(), 16);

        let err = lazy(|cx| payload.poll_stream(cx)).await.unwrap_err();
        assert!(matches!(err, PayloadError::Overflow));
        assert_eq!(payload.buf.len(), 16);
    }

    #[actix_rt::test]
    async fn oversized_chunk_can_be_consumed_incrementally() {
        let stream = futures_util::stream::once(async { Ok(Bytes::from_static(b"12345678")) });
        let mut payload = PayloadBuffer::new_with_limit(stream, 4);

        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(payload.buf, Bytes::from_static(b"1234"));
        assert_eq!(payload.read_max(4).unwrap().unwrap(), "1234");

        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert_eq!(payload.buf, Bytes::from_static(b"5678"));
        assert_eq!(payload.read_max(4).unwrap().unwrap(), "5678");

        lazy(|cx| payload.poll_stream(cx)).await.unwrap();
        assert!(payload.eof);
    }
}

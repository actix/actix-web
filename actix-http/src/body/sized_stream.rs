use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};

use crate::error::Error;

use super::{BodySize, MessageBody};

/// Known sized streaming response wrapper.
///
/// This body implementation should be used if total size of stream is known. Data get sent as is
/// without using transfer encoding.
pub struct SizedStream<S: Unpin> {
    size: u64,
    stream: S,
}

impl<S> SizedStream<S>
where
    S: Stream<Item = Result<Bytes, Error>> + Unpin,
{
    pub fn new(size: u64, stream: S) -> Self {
        SizedStream { size, stream }
    }
}

impl<S> MessageBody for SizedStream<S>
where
    S: Stream<Item = Result<Bytes, Error>> + Unpin,
{
    fn size(&self) -> BodySize {
        BodySize::Sized(self.size as u64)
    }

    /// Attempts to pull out the next value of the underlying [`Stream`].
    ///
    /// Empty values are skipped to prevent [`SizedStream`]'s transmission being
    /// ended on a zero-length chunk, but rather proceed until the underlying
    /// [`Stream`] ends.
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        loop {
            let stream = &mut self.as_mut().stream;

            let chunk = match ready!(Pin::new(stream).poll_next(cx)) {
                Some(Ok(ref bytes)) if bytes.is_empty() => continue,
                val => val,
            };

            return Poll::Ready(chunk);
        }
    }
}

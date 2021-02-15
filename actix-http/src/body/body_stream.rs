use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};

use crate::error::Error;

use super::{BodySize, MessageBody};

/// Streaming response wrapper.
///
/// Response does not contain `Content-Length` header and appropriate transfer encoding is used.
pub struct BodyStream<S: Unpin> {
    stream: S,
}

impl<S, E> BodyStream<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: Into<Error>,
{
    pub fn new(stream: S) -> Self {
        BodyStream { stream }
    }
}

impl<S, E> MessageBody for BodyStream<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: Into<Error>,
{
    fn size(&self) -> BodySize {
        BodySize::Stream
    }

    /// Attempts to pull out the next value of the underlying [`Stream`].
    ///
    /// Empty values are skipped to prevent [`BodyStream`]'s transmission being
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
                opt => opt.map(|res| res.map_err(Into::into)),
            };

            return Poll::Ready(chunk);
        }
    }
}

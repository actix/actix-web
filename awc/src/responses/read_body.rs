use std::{
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{error::PayloadError, Payload};
use bytes::{Bytes, BytesMut};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use super::response::DecodeLimitFn;

pin_project! {
    pub(crate) struct ReadBody<S> {
        #[pin]
        pub(crate) stream: Payload<S>,
        pub(crate) buf: BytesMut,
        pub(crate) limit: usize,
        decode_limit: Option<DecodeLimitFn<S>>,
    }
}

impl<S> ReadBody<S> {
    pub(crate) fn new(
        stream: Payload<S>,
        limit: usize,
        decode_limit: Option<DecodeLimitFn<S>>,
    ) -> Self {
        let mut body = Self {
            stream,
            buf: BytesMut::new(),
            limit,
            decode_limit,
        };

        body.set_limit(limit);
        body
    }

    /// Sets the size limit, also stopping decompression once decoded output exceeds it.
    ///
    /// Must be called before the body is first polled.
    pub(crate) fn set_limit(&mut self, limit: usize) {
        self.limit = limit;

        if let Some(decode_limit) = self.decode_limit {
            let stream = mem::replace(&mut self.stream, Payload::None);
            self.stream = decode_limit(stream, limit);
        }
    }
}

impl<S> Future for ReadBody<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    type Output = Result<Bytes, PayloadError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        while let Some(chunk) = ready!(this.stream.as_mut().poll_next(cx)?) {
            if (this.buf.len() + chunk.len()) > *this.limit {
                return Poll::Ready(Err(PayloadError::Overflow));
            }

            this.buf.extend_from_slice(&chunk);
        }

        Poll::Ready(Ok(this.buf.split().freeze()))
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::assert_impl_all;

    use super::*;
    use crate::any_body::AnyBody;

    assert_impl_all!(ReadBody<()>: Unpin);
    assert_impl_all!(ReadBody<AnyBody>: Unpin);
}

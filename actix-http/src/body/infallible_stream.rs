use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::Stream;
use pin_project_lite::pin_project;

pin_project! {
    /// Adapts a byte stream into a stream of results with an [`Infallible`] error.
    ///
    /// Created by [`BodyStream::from_infallible`](super::BodyStream::from_infallible) or
    /// [`SizedStream::from_infallible`](super::SizedStream::from_infallible).
    #[must_use]
    pub struct InfallibleStream<S> {
        #[pin]
        pub(super) stream: S,
    }
}

impl<S: Stream<Item = Bytes>> Stream for InfallibleStream<S> {
    type Item = Result<Bytes, Infallible>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project()
            .stream
            .poll_next(cx)
            .map(|chunk| chunk.map(Ok))
    }
}

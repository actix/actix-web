use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::Stream;
use pin_project_lite::pin_project;

pin_project! {
    /// Adapts a stream into a stream of results with an [`Infallible`] error.
    ///
    /// Created by [`BodyStream::from_infallible`](super::BodyStream::from_infallible) or
    /// [`SizedStream::from_infallible`](super::SizedStream::from_infallible).
    #[must_use]
    pub struct InfallibleStream<S> {
        #[pin]
        pub(super) stream: S,
    }
}

impl<S: Stream> Stream for InfallibleStream<S> {
    type Item = Result<S::Item, Infallible>;

    #[inline]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().stream.poll_next(cx).map(|item| item.map(Ok))
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt as _;

    use super::*;

    #[tokio::test(flavor = "local")]
    async fn non_byte_items() {
        #[derive(Debug, PartialEq)]
        struct Item(u8);

        let stream = async_stream::stream! {
            yield Item(1);
            tokio::task::yield_now().await;
            yield Item(2);
        };
        let stream = InfallibleStream { stream };

        assert_eq!(
            stream.collect::<Vec<_>>().await,
            vec![Ok(Item(1)), Ok(Item(2))],
        );
    }
}

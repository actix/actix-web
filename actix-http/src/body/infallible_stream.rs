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
    /// Created by [`InfallibleStream::new`],
    /// [`BodyStream::from_infallible`](super::BodyStream::from_infallible), or
    /// [`SizedStream::from_infallible`](super::SizedStream::from_infallible).
    #[must_use]
    pub struct InfallibleStream<S> {
        #[pin]
        pub(super) stream: S,
    }
}

impl<S: Stream> InfallibleStream<S> {
    /// Creates a stream that wraps each item from `stream` in `Ok`.
    ///
    /// # Examples
    ///
    /// ```
    /// use actix_http::body::InfallibleStream;
    /// use futures_util::{stream, StreamExt as _};
    ///
    /// # actix_rt::System::new().block_on(async {
    /// let stream = InfallibleStream::new(stream::iter([1, 2]));
    ///
    /// assert_eq!(stream.collect::<Vec<_>>().await, vec![Ok(1), Ok(2)]);
    /// # });
    /// ```
    #[inline]
    pub fn new(stream: S) -> Self {
        Self { stream }
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
        let stream = InfallibleStream::new(stream);

        assert_eq!(
            stream.collect::<Vec<_>>().await,
            vec![Ok(Item(1)), Ok(Item(2))],
        );
    }
}

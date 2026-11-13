use std::{
    error::Error as StdError,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use super::{BodySize, InfallibleStream, MessageBody};

pin_project! {
    /// Known sized streaming response wrapper.
    ///
    /// This body implementation should be used if total size of stream is known. Data is sent as-is
    /// without using chunked transfer encoding.
    pub struct SizedStream<S> {
        size: u64,
        #[pin]
        stream: S,
    }
}

impl<S, E> SizedStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn StdError>> + 'static,
{
    #[inline]
    #[must_use]
    pub fn new(size: u64, stream: S) -> Self {
        SizedStream { size, stream }
    }
}

impl<S> SizedStream<InfallibleStream<S>>
where
    S: Stream<Item = Bytes>,
{
    /// Creates a body from a stream of bytes that cannot yield errors.
    ///
    /// The caller must provide the total number of bytes in the stream.
    ///
    /// Empty chunks are skipped.
    ///
    /// # Examples
    ///
    /// ```
    /// # use actix_http::body::{SizedStream, InfallibleStream};
    /// # use bytes::Bytes;
    /// use futures_util::stream;
    ///
    /// let stream = stream::iter([Bytes::from_static(b"hello")]);
    /// let body = SizedStream::from_infallible(5, stream);
    /// ```
    #[inline]
    #[must_use]
    pub fn from_infallible(size: u64, stream: S) -> Self {
        SizedStream::new(size, InfallibleStream { stream })
    }
}

impl<S, E> MessageBody for SizedStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn StdError>> + 'static,
{
    type Error = E;

    #[inline]
    fn size(&self) -> BodySize {
        BodySize::Sized(self.size)
    }

    /// Attempts to pull out the next value of the underlying [`Stream`].
    ///
    /// Empty values are skipped to prevent [`SizedStream`]'s transmission being
    /// ended on a zero-length chunk, but rather proceed until the underlying
    /// [`Stream`] ends.
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        loop {
            let stream = self.as_mut().project().stream;

            let chunk = match ready!(stream.poll_next(cx)) {
                Some(Ok(ref bytes)) if bytes.is_empty() => continue,
                val => val,
            };

            return Poll::Ready(chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, pin::pin};

    use actix_utils::future::poll_fn;
    use futures_util::stream;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;
    use crate::body::to_bytes;

    assert_impl_all!(SizedStream<stream::Empty<Result<Bytes, crate::Error>>>: MessageBody);
    assert_impl_all!(SizedStream<stream::Empty<Result<Bytes, &'static str>>>: MessageBody);
    assert_impl_all!(SizedStream<stream::Repeat<Result<Bytes, &'static str>>>: MessageBody);
    assert_impl_all!(SizedStream<stream::Empty<Result<Bytes, Infallible>>>: MessageBody);
    assert_impl_all!(SizedStream<stream::Repeat<Result<Bytes, Infallible>>>: MessageBody);

    assert_not_impl_any!(SizedStream<stream::Empty<Bytes>>: MessageBody);
    assert_not_impl_any!(SizedStream<stream::Repeat<Bytes>>: MessageBody);
    // crate::Error is not Clone
    assert_not_impl_any!(SizedStream<stream::Repeat<Result<Bytes, crate::Error>>>: MessageBody);

    #[tokio::test(flavor = "local")]
    async fn infallible_stream() {
        let stream = async_stream::stream! {
            yield Bytes::new();
            yield Bytes::from_static(b"1");
            tokio::task::yield_now().await;
            yield Bytes::new();
            yield Bytes::from_static(b"2");
            yield Bytes::new();
        };
        let body = SizedStream::from_infallible(2, stream);

        assert_eq!(body.size(), BodySize::Sized(2));
        assert_eq!(
            to_bytes(body).await,
            Ok::<_, Infallible>(Bytes::from_static(b"12"))
        );
    }

    #[tokio::test(flavor = "local")]
    async fn infallible_empty_stream() {
        let body = SizedStream::from_infallible(0, stream::empty::<Bytes>());

        assert_eq!(to_bytes(body).await, Ok::<_, Infallible>(Bytes::new()));
    }

    #[tokio::test(flavor = "local")]
    async fn skips_empty_chunks() {
        let body = SizedStream::new(
            2,
            stream::iter(
                ["1", "", "2"]
                    .iter()
                    .map(|&v| Ok::<_, Infallible>(Bytes::from(v))),
            ),
        );

        let mut body = pin!(body);

        assert_eq!(
            poll_fn(|cx| body.as_mut().poll_next(cx))
                .await
                .unwrap()
                .ok(),
            Some(Bytes::from("1")),
        );

        assert_eq!(
            poll_fn(|cx| body.as_mut().poll_next(cx))
                .await
                .unwrap()
                .ok(),
            Some(Bytes::from("2")),
        );
    }

    #[tokio::test(flavor = "local")]
    async fn read_to_bytes() {
        let body = SizedStream::new(
            2,
            stream::iter(
                ["1", "", "2"]
                    .iter()
                    .map(|&v| Ok::<_, Infallible>(Bytes::from(v))),
            ),
        );

        assert_eq!(to_bytes(body).await.ok(), Some(Bytes::from("12")));
    }

    #[tokio::test(flavor = "local")]
    async fn stream_string_error() {
        // `&'static str` does not impl `Error`
        // but it does impl `Into<Box<dyn Error>>`

        let body = SizedStream::new(0, stream::once(async { Err("stringy error") }));
        assert_eq!(to_bytes(body).await, Ok(Bytes::new()));

        let body = SizedStream::new(1, stream::once(async { Err("stringy error") }));
        assert!(matches!(to_bytes(body).await, Err("stringy error")));
    }

    #[tokio::test(flavor = "local")]
    async fn stream_boxed_error() {
        // `Box<dyn Error>` does not impl `Error`
        // but it does impl `Into<Box<dyn Error>>`

        let body = SizedStream::new(
            0,
            stream::once(async { Err(Box::<dyn StdError>::from("stringy error")) }),
        );
        assert_eq!(to_bytes(body).await.unwrap(), Bytes::new());

        let body = SizedStream::new(
            1,
            stream::once(async { Err(Box::<dyn StdError>::from("stringy error")) }),
        );
        assert_eq!(
            to_bytes(body).await.unwrap_err().to_string(),
            "stringy error"
        );
    }
}

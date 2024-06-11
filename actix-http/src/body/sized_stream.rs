use std::{
    error::Error as StdError,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use super::{BodySize, MessageBody};

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
    pub fn new(size: u64, stream: S) -> Self {
        SizedStream { size, stream }
    }
}

// TODO: from_infallible method

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
    use std::convert::Infallible;

    use actix_rt::pin;
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

    #[actix_rt::test]
    async fn skips_empty_chunks() {
        let body = SizedStream::new(
            2,
            stream::iter(
                ["1", "", "2"]
                    .iter()
                    .map(|&v| Ok::<_, Infallible>(Bytes::from(v))),
            ),
        );

        pin!(body);

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

    #[actix_rt::test]
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

    #[actix_rt::test]
    async fn stream_string_error() {
        // `&'static str` does not impl `Error`
        // but it does impl `Into<Box<dyn Error>>`

        let body = SizedStream::new(0, stream::once(async { Err("stringy error") }));
        assert_eq!(to_bytes(body).await, Ok(Bytes::new()));

        let body = SizedStream::new(1, stream::once(async { Err("stringy error") }));
        assert!(matches!(to_bytes(body).await, Err("stringy error")));
    }

    #[actix_rt::test]
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

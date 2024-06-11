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
    /// Streaming response wrapper.
    ///
    /// Response does not contain `Content-Length` header and appropriate transfer encoding is used.
    pub struct BodyStream<S> {
        #[pin]
        stream: S,
    }
}

// TODO: from_infallible method

impl<S, E> BodyStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn StdError>> + 'static,
{
    #[inline]
    pub fn new(stream: S) -> Self {
        BodyStream { stream }
    }
}

impl<S, E> MessageBody for BodyStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn StdError>> + 'static,
{
    type Error = E;

    #[inline]
    fn size(&self) -> BodySize {
        BodySize::Stream
    }

    /// Attempts to pull out the next value of the underlying [`Stream`].
    ///
    /// Empty values are skipped to prevent [`BodyStream`]'s transmission being ended on a
    /// zero-length chunk, but rather proceed until the underlying [`Stream`] ends.
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        loop {
            let stream = self.as_mut().project().stream;

            let chunk = match ready!(stream.poll_next(cx)) {
                Some(Ok(ref bytes)) if bytes.is_empty() => continue,
                opt => opt,
            };

            return Poll::Ready(chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, time::Duration};

    use actix_rt::{
        pin,
        time::{sleep, Sleep},
    };
    use actix_utils::future::poll_fn;
    use derive_more::{Display, Error};
    use futures_core::ready;
    use futures_util::{stream, FutureExt as _};
    use pin_project_lite::pin_project;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;
    use crate::body::to_bytes;

    assert_impl_all!(BodyStream<stream::Empty<Result<Bytes, crate::Error>>>: MessageBody);
    assert_impl_all!(BodyStream<stream::Empty<Result<Bytes, &'static str>>>: MessageBody);
    assert_impl_all!(BodyStream<stream::Repeat<Result<Bytes, &'static str>>>: MessageBody);
    assert_impl_all!(BodyStream<stream::Empty<Result<Bytes, Infallible>>>: MessageBody);
    assert_impl_all!(BodyStream<stream::Repeat<Result<Bytes, Infallible>>>: MessageBody);

    assert_not_impl_any!(BodyStream<stream::Empty<Bytes>>: MessageBody);
    assert_not_impl_any!(BodyStream<stream::Repeat<Bytes>>: MessageBody);
    // crate::Error is not Clone
    assert_not_impl_any!(BodyStream<stream::Repeat<Result<Bytes, crate::Error>>>: MessageBody);

    #[actix_rt::test]
    async fn skips_empty_chunks() {
        let body = BodyStream::new(stream::iter(
            ["1", "", "2"]
                .iter()
                .map(|&v| Ok::<_, Infallible>(Bytes::from(v))),
        ));
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
        let body = BodyStream::new(stream::iter(
            ["1", "", "2"]
                .iter()
                .map(|&v| Ok::<_, Infallible>(Bytes::from(v))),
        ));

        assert_eq!(to_bytes(body).await.ok(), Some(Bytes::from("12")));
    }
    #[derive(Debug, Display, Error)]
    #[display(fmt = "stream error")]
    struct StreamErr;

    #[actix_rt::test]
    async fn stream_immediate_error() {
        let body = BodyStream::new(stream::once(async { Err(StreamErr) }));
        assert!(matches!(to_bytes(body).await, Err(StreamErr)));
    }

    #[actix_rt::test]
    async fn stream_string_error() {
        // `&'static str` does not impl `Error`
        // but it does impl `Into<Box<dyn Error>>`

        let body = BodyStream::new(stream::once(async { Err("stringy error") }));
        assert!(matches!(to_bytes(body).await, Err("stringy error")));
    }

    #[actix_rt::test]
    async fn stream_boxed_error() {
        // `Box<dyn Error>` does not impl `Error`
        // but it does impl `Into<Box<dyn Error>>`

        let body = BodyStream::new(stream::once(async {
            Err(Box::<dyn StdError>::from("stringy error"))
        }));

        assert_eq!(
            to_bytes(body).await.unwrap_err().to_string(),
            "stringy error"
        );
    }

    #[actix_rt::test]
    async fn stream_delayed_error() {
        let body = BodyStream::new(stream::iter(vec![Ok(Bytes::from("1")), Err(StreamErr)]));
        assert!(matches!(to_bytes(body).await, Err(StreamErr)));

        pin_project! {
            #[derive(Debug)]
            #[project = TimeDelayStreamProj]
            enum TimeDelayStream {
                Start,
                Sleep { delay: Pin<Box<Sleep>> },
                Done,
            }
        }

        impl Stream for TimeDelayStream {
            type Item = Result<Bytes, StreamErr>;

            fn poll_next(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Option<Self::Item>> {
                match self.as_mut().get_mut() {
                    TimeDelayStream::Start => {
                        let sleep = sleep(Duration::from_millis(1));
                        self.as_mut().set(TimeDelayStream::Sleep {
                            delay: Box::pin(sleep),
                        });
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }

                    TimeDelayStream::Sleep { ref mut delay } => {
                        ready!(delay.poll_unpin(cx));
                        self.set(TimeDelayStream::Done);
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }

                    TimeDelayStream::Done => Poll::Ready(Some(Err(StreamErr))),
                }
            }
        }

        let body = BodyStream::new(TimeDelayStream::Start);
        assert!(matches!(to_bytes(body).await, Err(StreamErr)));
    }
}

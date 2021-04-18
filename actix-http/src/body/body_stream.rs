use std::{
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use crate::error::Error;

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

impl<S, E> BodyStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Error>,
{
    pub fn new(stream: S) -> Self {
        BodyStream { stream }
    }
}

impl<S, E> MessageBody for BodyStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
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
            let stream = self.as_mut().project().stream;

            let chunk = match ready!(stream.poll_next(cx)) {
                Some(Ok(ref bytes)) if bytes.is_empty() => continue,
                opt => opt.map(|res| res.map_err(Into::into)),
            };

            return Poll::Ready(chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use actix_rt::pin;
    use actix_utils::future::poll_fn;
    use futures_util::stream;

    use super::*;
    use crate::body::to_bytes;

    #[actix_rt::test]
    async fn skips_empty_chunks() {
        let body = BodyStream::new(stream::iter(
            ["1", "", "2"]
                .iter()
                .map(|&v| Ok(Bytes::from(v)) as Result<Bytes, ()>),
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
                .map(|&v| Ok(Bytes::from(v)) as Result<Bytes, ()>),
        ));

        assert_eq!(to_bytes(body).await.ok(), Some(Bytes::from("12")));
    }
}

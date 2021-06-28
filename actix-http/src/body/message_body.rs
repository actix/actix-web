//! [`MessageBody`] trait and foreign implementations.

use std::{
    convert::Infallible,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_core::ready;
use pin_project_lite::pin_project;

use crate::error::Error;

use super::BodySize;

/// An interface for response bodies.
pub trait MessageBody {
    type Error;

    /// Body size hint.
    fn size(&self) -> BodySize;

    /// Attempt to pull out the next chunk of body bytes.
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>>;
}

impl MessageBody for () {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Empty
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        Poll::Ready(None)
    }
}

impl<B> MessageBody for Box<B>
where
    B: MessageBody + Unpin,
    B::Error: Into<Error>,
{
    type Error = B::Error;

    fn size(&self) -> BodySize {
        self.as_ref().size()
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        Pin::new(self.get_mut().as_mut()).poll_next(cx)
    }
}

impl<B> MessageBody for Pin<Box<B>>
where
    B: MessageBody,
    B::Error: Into<Error>,
{
    type Error = B::Error;

    fn size(&self) -> BodySize {
        self.as_ref().size()
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        self.as_mut().poll_next(cx)
    }
}

impl MessageBody for Bytes {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(mem::take(self.get_mut()))))
        }
    }
}

impl MessageBody for BytesMut {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(mem::take(self.get_mut()).freeze())))
        }
    }
}

impl MessageBody for &'static str {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from_static(
                mem::take(self.get_mut()).as_ref(),
            ))))
        }
    }
}

impl MessageBody for Vec<u8> {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from(mem::take(self.get_mut())))))
        }
    }
}

impl MessageBody for String {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from(
                mem::take(self.get_mut()).into_bytes(),
            ))))
        }
    }
}

pin_project! {
    pub(crate) struct MessageBodyMapErr<B, F> {
        #[pin]
        body: B,
        mapper: Option<F>,
    }
}

impl<B, F, E> MessageBodyMapErr<B, F>
where
    B: MessageBody,
    F: FnOnce(B::Error) -> E,
{
    pub(crate) fn new(body: B, mapper: F) -> Self {
        Self {
            body,
            mapper: Some(mapper),
        }
    }
}

impl<B, F, E> MessageBody for MessageBodyMapErr<B, F>
where
    B: MessageBody,
    F: FnOnce(B::Error) -> E,
{
    type Error = E;

    fn size(&self) -> BodySize {
        self.body.size()
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        let this = self.as_mut().project();

        match ready!(this.body.poll_next(cx)) {
            Some(Err(err)) => {
                let f = self.as_mut().project().mapper.take().unwrap();
                let mapped_err = (f)(err);
                Poll::Ready(Some(Err(mapped_err)))
            }
            Some(Ok(val)) => Poll::Ready(Some(Ok(val))),
            None => Poll::Ready(None),
        }
    }
}

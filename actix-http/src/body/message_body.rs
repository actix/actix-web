//! [`MessageBody`] trait and foreign implementations.

use std::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};

use crate::error::Error;

use super::BodySize;

/// An interface for response bodies.
pub trait MessageBody {
    /// Body size hint.
    fn size(&self) -> BodySize;

    /// Attempt to pull out the next chunk of body bytes.
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>>;

    downcast_get_type_id!();
}

downcast!(MessageBody);

impl MessageBody for () {
    fn size(&self) -> BodySize {
        BodySize::Empty
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        Poll::Ready(None)
    }
}

impl<T: MessageBody + Unpin> MessageBody for Box<T> {
    fn size(&self) -> BodySize {
        self.as_ref().size()
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        Pin::new(self.get_mut().as_mut()).poll_next(cx)
    }
}

impl<T: MessageBody> MessageBody for Pin<Box<T>> {
    fn size(&self) -> BodySize {
        self.as_ref().size()
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        self.as_mut().poll_next(cx)
    }
}

impl MessageBody for Bytes {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(mem::take(self.get_mut()))))
        }
    }
}

impl MessageBody for BytesMut {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(mem::take(self.get_mut()).freeze())))
        }
    }
}

impl MessageBody for &'static str {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
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
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from(mem::take(self.get_mut())))))
        }
    }
}

impl MessageBody for String {
    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from(
                mem::take(self.get_mut()).into_bytes(),
            ))))
        }
    }
}

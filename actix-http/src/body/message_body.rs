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

use super::BodySize;

/// An interface types that can converted to bytes and used as response bodies.
// TODO: examples
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

impl MessageBody for Infallible {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        match *self {}
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match *self {}
    }
}

impl MessageBody for () {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(0)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        Poll::Ready(None)
    }
}

impl<B> MessageBody for Box<B>
where
    B: MessageBody + Unpin,
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

impl MessageBody for &'static [u8] {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            let bytes = mem::take(self.get_mut());
            let bytes = Bytes::from_static(bytes);
            Poll::Ready(Some(Ok(bytes)))
        }
    }
}

impl MessageBody for Bytes {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            let bytes = mem::take(self.get_mut());
            Poll::Ready(Some(Ok(bytes)))
        }
    }
}

// impl<'a> MessageBody for &'a Bytes {
//     type Error = Infallible;

//     fn size(&self) -> BodySize {
//         BodySize::Sized(self.len() as u64)
//     }

//     fn poll_next(
//         self: Pin<&mut Self>,
//         _cx: &mut Context<'_>,
//     ) -> Poll<Option<Result<Bytes, Self::Error>>> {
//         if self.is_empty() {
//             Poll::Ready(None)
//         } else {
//             Poll::Ready(Some(Ok(self.clone())))
//         }
//     }
// }

impl MessageBody for BytesMut {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(mem::take(self.get_mut()).freeze())))
        }
    }
}

// impl<'a> MessageBody for &'a BytesMut {
//     type Error = Infallible;

//     fn size(&self) -> BodySize {
//         BodySize::Sized(self.len() as u64)
//     }

//     fn poll_next(
//         self: Pin<&mut Self>,
//         _cx: &mut Context<'_>,
//     ) -> Poll<Option<Result<Bytes, Self::Error>>> {
//         if self.is_empty() {
//             Poll::Ready(None)
//         } else {
//             Poll::Ready(Some(Ok(self.clone().freeze())))
//         }
//     }
// }

impl MessageBody for &'static str {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            let string = mem::take(self.get_mut());
            let bytes = Bytes::from_static(string.as_bytes());
            Poll::Ready(Some(Ok(bytes)))
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
        _cx: &mut Context<'_>,
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
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        if self.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Ready(Some(Ok(Bytes::from(mem::take(self.get_mut())))))
        }
    }
}

// impl<'a> MessageBody for &'a String {
//     type Error = Infallible;

//     fn size(&self) -> BodySize {
//         BodySize::Sized(self.len() as u64)
//     }

//     fn poll_next(
//         self: Pin<&mut Self>,
//         _cx: &mut Context<'_>,
//     ) -> Poll<Option<Result<Bytes, Self::Error>>> {
//         if self.is_empty() {
//             Poll::Ready(None)
//         } else {
//             Poll::Ready(Some(Ok(Bytes::from(self.clone()))))
//         }
//     }
// }

// impl MessageBody for Cow<'_, str> {
//     type Error = Infallible;

//     fn size(&self) -> BodySize {
//         BodySize::Sized(self.len() as u64)
//     }

//     fn poll_next(
//         self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//     ) -> Poll<Option<Result<Bytes, Self::Error>>> {
//         if self.is_empty() {
//             Poll::Ready(None)
//         } else {
//             let cow = Pin::into_inner(self);
//             let mut string = cow.clone().into_owned();
//             Pin::new(&mut string).poll_next(cx)
//         }
//     }
// }

impl MessageBody for bytestring::ByteString {
    type Error = Infallible;

    fn size(&self) -> BodySize {
        BodySize::Sized(self.len() as u64)
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        let string = mem::take(self.get_mut());
        Poll::Ready(Some(Ok(string.into_bytes())))
    }
}

// TODO: ensure consistent impls of MessageBody that always terminate

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

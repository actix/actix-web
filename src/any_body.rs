use std::{
    error::Error as StdError,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::body::{BodySize, BoxBody, MessageBody};
use bytes::Bytes;
use pin_project_lite::pin_project;

use crate::Error;

pin_project! {
    #[derive(Debug)]
    #[project = AnyBodyProj]
    pub enum AnyBody<B = BoxBody> {
        None,
        Full { body: Bytes },
        Stream { #[pin] body: B },
        Boxed { body: BoxBody },
    }
}

impl<B: MessageBody + 'static> AnyBody<B> {
    pub fn into_body<B1>(self) -> AnyBody<B1> {
        match self {
            AnyBody::None => AnyBody::None,
            AnyBody::Full { body } => AnyBody::Full { body },
            AnyBody::Stream { body } => AnyBody::Boxed {
                body: BoxBody::new(body),
            },
            AnyBody::Boxed { body } => AnyBody::Boxed { body },
        }
    }
}

impl<B> Default for AnyBody<B> {
    fn default() -> Self {
        Self::Full { body: Bytes::new() }
    }
}

impl<B> MessageBody for AnyBody<B>
where
    B: MessageBody,
    B::Error: 'static,
{
    type Error = Box<dyn StdError>;

    fn size(&self) -> BodySize {
        match self {
            Self::None => BodySize::None,
            Self::Full { body } => body.size(),
            Self::Stream { body } => body.size(),
            Self::Boxed { body } => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.project() {
            AnyBodyProj::None => Poll::Ready(None),
            AnyBodyProj::Full { body } => {
                let bytes = mem::take(body);
                Poll::Ready(Some(Ok(bytes)))
            }
            AnyBodyProj::Stream { body } => body.poll_next(cx).map_err(|err| err.into()),
            AnyBodyProj::Boxed { body } => body.as_pin_mut().poll_next(cx),
        }
    }
}

pin_project! {
    #[project = EitherAnyBodyProj]
    #[derive(Debug)]
    pub enum EitherAnyBody<L, R = BoxBody> {
        /// A body of type `L`.
        Left { #[pin] body: AnyBody<L> },

        /// A body of type `R`.
        Right { #[pin] body: AnyBody<R> },
    }
}

// impl<L> EitherAnyBody<L, BoxBody> {
//     /// Creates new `EitherBody` using left variant and boxed right variant.
//     pub fn new(body: L) -> Self {
//         Self::Left {
//             body: AnyBody::Stream { body },
//         }
//     }
// }

// impl<L, R> EitherAnyBody<L, R> {
//     /// Creates new `EitherBody` using left variant.
//     pub fn left(body: L) -> Self {
//         Self::Left {
//             body: AnyBody::Stream { body },
//         }
//     }

//     /// Creates new `EitherBody` using right variant.
//     pub fn right(body: R) -> Self {
//         Self::Right {
//             body: AnyBody::Stream { body },
//         }
//     }
// }

impl<L, R> MessageBody for EitherAnyBody<L, R>
where
    L: MessageBody + 'static,
    R: MessageBody + 'static,
{
    type Error = Error;

    fn size(&self) -> BodySize {
        match self {
            Self::Left { body } => body.size(),
            Self::Right { body } => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.project() {
            EitherAnyBodyProj::Left { body } => body.poll_next(cx).map_err(Error::from),
            EitherAnyBodyProj::Right { body } => body.poll_next(cx).map_err(Error::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::assert_eq_size;

    use super::*;

    assert_eq_size!(AnyBody<()>, [u8; 40]);
    assert_eq_size!(AnyBody<u64>, [u8; 40]); // how is this the same size as ()
}

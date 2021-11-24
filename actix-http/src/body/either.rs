use std::{
    error::Error as StdError,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use pin_project_lite::pin_project;

use super::{BodySize, BoxBody, MessageBody};
use crate::Error;

pin_project! {
    #[project = EitherBodyProj]
    #[derive(Debug, Clone)]
    pub enum EitherBody<L, R = BoxBody> {
        /// A body of type `L`.
        Left { #[pin] body: L },

        /// A body of type `R`.
        Right { #[pin] body: R },
    }
}

impl<L> EitherBody<L, BoxBody> {
    /// Creates new `EitherBody` using left variant and boxed right variant.
    pub fn new(body: L) -> Self {
        Self::Left { body }
    }
}

impl<L, R> EitherBody<L, R> {
    /// Creates new `EitherBody` using left variant.
    pub fn left(body: L) -> Self {
        Self::Left { body }
    }

    /// Creates new `EitherBody` using right variant.
    pub fn right(body: R) -> Self {
        Self::Right { body }
    }
}

impl<L, R> MessageBody for EitherBody<L, R>
where
    L: MessageBody + 'static,
    L::Error: Into<Box<dyn StdError + 'static>>,
    R: MessageBody + 'static,
    R::Error: Into<Box<dyn StdError + 'static>>,
{
    type Error = Error;

    fn size(&self) -> BodySize {
        match self {
            EitherBody::Left { body } => body.size(),
            EitherBody::Right { body } => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.project() {
            EitherBodyProj::Left { body } => body
                .poll_next(cx)
                .map_err(|err| Error::new_body().with_cause(err)),
            EitherBodyProj::Right { body } => body
                .poll_next(cx)
                .map_err(|err| Error::new_body().with_cause(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_body_works() {
        let body = EitherBody::new(());
    }
}

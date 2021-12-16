use std::{
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
    #[inline]
    pub fn new(body: L) -> Self {
        Self::Left { body }
    }
}

impl<L, R> EitherBody<L, R> {
    /// Creates new `EitherBody` using left variant.
    #[inline]
    pub fn left(body: L) -> Self {
        Self::Left { body }
    }

    /// Creates new `EitherBody` using right variant.
    #[inline]
    pub fn right(body: R) -> Self {
        Self::Right { body }
    }
}

impl<L, R> MessageBody for EitherBody<L, R>
where
    L: MessageBody + 'static,
    R: MessageBody + 'static,
{
    type Error = Error;

    #[inline]
    fn size(&self) -> BodySize {
        match self {
            EitherBody::Left { body } => body.size(),
            EitherBody::Right { body } => body.size(),
        }
    }

    #[inline]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.project() {
            EitherBodyProj::Left { body } => body
                .poll_next(cx)
                .map_err(|err| Error::new_body().with_cause(format!("{:?}", err))),
            EitherBodyProj::Right { body } => body
                .poll_next(cx)
                .map_err(|err| Error::new_body().with_cause(format!("{:?}", err))),
        }
    }

    #[inline]
    fn is_complete_body(&self) -> bool {
        match self {
            EitherBody::Left { body } => body.is_complete_body(),
            EitherBody::Right { body } => body.is_complete_body(),
        }
    }

    #[inline]
    fn take_complete_body(&mut self) -> Bytes {
        match self {
            EitherBody::Left { body } => body.take_complete_body(),
            EitherBody::Right { body } => body.take_complete_body(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_parameter_inference() {
        let _body: EitherBody<(), _> = EitherBody::new(());

        let _body: EitherBody<_, ()> = EitherBody::left(());
        let _body: EitherBody<(), _> = EitherBody::right(());
    }
}

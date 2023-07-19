use std::{
    error::Error as StdError,
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use super::{BodySize, MessageBody, MessageBodyMapErr};
use crate::body;

/// A boxed message body with boxed errors.
#[derive(Debug)]
pub struct BoxBody(BoxBodyInner);

enum BoxBodyInner {
    None(body::None),
    Bytes(Bytes),
    Stream(Pin<Box<dyn MessageBody<Error = Box<dyn StdError>>>>),
}

impl fmt::Debug for BoxBodyInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None(arg0) => f.debug_tuple("None").field(arg0).finish(),
            Self::Bytes(arg0) => f.debug_tuple("Bytes").field(arg0).finish(),
            Self::Stream(_) => f.debug_tuple("Stream").field(&"dyn MessageBody").finish(),
        }
    }
}

impl BoxBody {
    /// Boxes body type, erasing type information.
    ///
    /// If the body type to wrap is unknown or generic it is better to use [`MessageBody::boxed`] to
    /// avoid double boxing.
    #[inline]
    pub fn new<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
    {
        match body.size() {
            BodySize::None => Self(BoxBodyInner::None(body::None)),
            _ => match body.try_into_bytes() {
                Ok(bytes) => Self(BoxBodyInner::Bytes(bytes)),
                Err(body) => {
                    let body = MessageBodyMapErr::new(body, Into::into);
                    Self(BoxBodyInner::Stream(Box::pin(body)))
                }
            },
        }
    }

    /// Returns a mutable pinned reference to the inner message body type.
    #[inline]
    pub fn as_pin_mut(&mut self) -> Pin<&mut Self> {
        Pin::new(self)
    }
}

impl MessageBody for BoxBody {
    type Error = Box<dyn StdError>;

    #[inline]
    fn size(&self) -> BodySize {
        match &self.0 {
            BoxBodyInner::None(none) => none.size(),
            BoxBodyInner::Bytes(bytes) => bytes.size(),
            BoxBodyInner::Stream(stream) => stream.size(),
        }
    }

    #[inline]
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match &mut self.0 {
            BoxBodyInner::None(body) => Pin::new(body).poll_next(cx).map_err(|err| match err {}),
            BoxBodyInner::Bytes(body) => Pin::new(body).poll_next(cx).map_err(|err| match err {}),
            BoxBodyInner::Stream(body) => Pin::new(body).poll_next(cx),
        }
    }

    #[inline]
    fn try_into_bytes(self) -> Result<Bytes, Self> {
        match self.0 {
            BoxBodyInner::None(body) => Ok(body.try_into_bytes().unwrap()),
            BoxBodyInner::Bytes(body) => Ok(body.try_into_bytes().unwrap()),
            _ => Err(self),
        }
    }

    #[inline]
    fn boxed(self) -> BoxBody {
        self
    }
}

#[cfg(test)]
mod tests {
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;
    use crate::body::to_bytes;

    assert_impl_all!(BoxBody: fmt::Debug, MessageBody, Unpin);
    assert_not_impl_any!(BoxBody: Send, Sync);

    #[actix_rt::test]
    async fn nested_boxed_body() {
        let body = Bytes::from_static(&[1, 2, 3]);
        let boxed_body = BoxBody::new(BoxBody::new(body));

        assert_eq!(
            to_bytes(boxed_body).await.unwrap(),
            Bytes::from(vec![1, 2, 3]),
        );
    }
}

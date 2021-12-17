use std::{
    error::Error as StdError,
    fmt, mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use super::{BodySize, MessageBody, MessageBodyMapErr};
use crate::Error;

/// A boxed message body with boxed errors.
pub struct BoxBody(BoxBodyInner);

enum BoxBodyInner {
    None,
    Bytes(Bytes),
    Stream(Pin<Box<dyn MessageBody<Error = Box<dyn StdError>>>>),
}

impl BoxBody {
    /// Same as `MessageBody::boxed`.
    ///
    /// If the body type to wrap is unknown or generic it is better to use [`MessageBody::boxed`] to
    /// avoid double boxing.
    #[inline]
    pub fn new<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
    {
        match body.size() {
            BodySize::None => Self(BoxBodyInner::None),
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
    //pub fn as_pin_mut(&mut self) -> Pin<&mut (dyn MessageBody<Error = Box<dyn StdError>>)> {
    pub fn as_pin_mut(&mut self) -> Pin<&mut Self> {
        Pin::new(self)
    }
}

impl fmt::Debug for BoxBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO show BoxBodyInner
        f.write_str("BoxBody(dyn MessageBody)")
    }
}

impl MessageBody for BoxBody {
    type Error = Error;

    #[inline]
    fn size(&self) -> BodySize {
        match &self.0 {
            BoxBodyInner::None => BodySize::None,
            BoxBodyInner::Bytes(bytes) => BodySize::Sized(bytes.len() as u64),
            BoxBodyInner::Stream(stream) => stream.size(),
        }
    }

    #[inline]
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match &mut self.0 {
            BoxBodyInner::None => Poll::Ready(None),
            BoxBodyInner::Bytes(bytes) => Poll::Ready(Some(Ok(mem::take(bytes)))),
            BoxBodyInner::Stream(stream) => Pin::new(stream).poll_next(cx).map_err(|err| Error::new_body().with_cause(err)),
        }
    }

    #[inline]
    fn try_into_bytes(self) -> Result<Bytes, Self> {
        match self.0 {
            BoxBodyInner::Bytes(bytes) => Ok(bytes),
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

    use static_assertions::{assert_impl_all, assert_not_impl_all};

    use super::*;
    use crate::body::to_bytes;

    assert_impl_all!(BoxBody: MessageBody, fmt::Debug, Unpin);

    assert_not_impl_all!(BoxBody: Send, Sync, Unpin);

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

use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use super::{BodySize, MessageBody, MessageBodyMapErr};

/// A boxed message body with boxed errors.
pub struct BoxBody(Pin<Box<dyn MessageBody<Error = Box<dyn fmt::Debug>>>>);

impl BoxBody {
    /// Boxes a `MessageBody` and any errors it generates.
    ///
    /// If the body type to wrap is unknown or generic it is better to use [`MessageBody::boxed`] to
    /// avoid double boxing.
    #[inline]
    pub fn new<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
    {
        fn box_it<T: fmt::Debug + 'static>(it: T) -> Box<dyn fmt::Debug> {
            Box::new(it)
        }

        let body = MessageBodyMapErr::new(body, box_it);
        Self(Box::pin(body))
    }

    /// Returns a mutable pinned reference to the inner message body type.
    #[inline]
    pub fn as_pin_mut(&mut self) -> Pin<&mut (dyn MessageBody<Error = Box<dyn StdError>>)> {
        self.0.as_mut()
    }
}

impl fmt::Debug for BoxBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BoxBody(dyn MessageBody)")
    }
}

impl MessageBody for BoxBody {
    type Error = Box<dyn fmt::Debug>;

    #[inline]
    fn size(&self) -> BodySize {
        self.0.size()
    }

    #[inline]
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        self.0.as_mut().poll_next(cx)
    }

    #[inline]
    fn is_complete_body(&self) -> bool {
        self.0.is_complete_body()
    }

    #[inline]
    fn take_complete_body(&mut self) -> Bytes {
        self.0.take_complete_body()
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

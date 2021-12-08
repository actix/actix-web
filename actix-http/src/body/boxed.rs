use std::{
    error::Error as StdError,
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use super::{BodySize, MessageBody, MessageBodyMapErr};
use crate::Error;

/// A boxed message body with boxed errors.
pub struct BoxBody(Pin<Box<dyn MessageBody<Error = Box<dyn StdError>>>>);

impl BoxBody {
    /// Boxes a `MessageBody` and any errors it generates.
    pub fn new<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
    {
        let body = MessageBodyMapErr::new(body, Into::into);
        Self(Box::pin(body))
    }

    /// Returns a mutable pinned reference to the inner message body type.
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
    type Error = Error;

    fn size(&self) -> BodySize {
        self.0.size()
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        self.0
            .as_mut()
            .poll_next(cx)
            .map_err(|err| Error::new_body().with_cause(err))
    }

    fn is_complete_body(&self) -> bool {
        let a = self.0.is_complete_body();
        eprintln!("BoxBody is complete?: {}", a);
        a
    }

    fn take_complete_body(&mut self) -> Bytes {
        eprintln!("taking box body contents");

        debug_assert!(
            self.is_complete_body(),
            "boxed type does not allow taking complete body; caller should make sure to \
            call `is_complete_body` first",
        );

        // we do not have DerefMut access to call take_complete_body directly but since
        // is_complete_body is true we should expect the entire bytes chunk in one poll_next

        let waker = futures_util::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match self.as_pin_mut().poll_next(&mut cx) {
            Poll::Ready(Some(Ok(data))) => data,
            _ => {
                panic!(
                    "boxed type indicated it allows taking complete body but failed to \
                    return Bytes when polled",
                );
            }
        }
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

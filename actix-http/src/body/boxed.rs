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
        B::Error: Into<Box<dyn StdError + 'static>>,
    {
        let body = MessageBodyMapErr::new(body, Into::into);
        Self(Box::pin(body))
    }

    /// Returns a mutable pinned reference to the inner message body type.
    pub fn as_pin_mut(
        &mut self,
    ) -> Pin<&mut (dyn MessageBody<Error = Box<dyn StdError>>)> {
        self.0.as_mut()
    }
}

impl fmt::Debug for BoxBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BoxAnyBody(dyn MessageBody)")
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
        let body = Bytes::from(&[1, 2, 3]);
        let boxed_body = BoxBody::new(BoxBody::new(body));

        assert_eq!(
            to_bytes(boxed_body).await.unwrap(),
            Bytes::from(vec![1, 2, 3]),
        );
    }
}

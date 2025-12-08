use std::{
    fmt, mem,
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::body::{BodySize, BoxBody, MessageBody};
use bytes::Bytes;
use pin_project_lite::pin_project;

pin_project! {
    /// Represents various types of HTTP message body.
    #[derive(Clone)]
    #[project = AnyBodyProj]
    pub enum AnyBody<B = BoxBody> {
        /// Empty response. `Content-Length` header is not set.
        None,

        /// Complete, in-memory response body.
        Bytes { body: Bytes },

        /// Generic / Other message body.
        Body { #[pin] body: B },
    }
}

impl AnyBody {
    /// Constructs a "body" representing an empty response.
    pub fn none() -> Self {
        Self::None
    }

    /// Constructs a new, 0-length body.
    pub fn empty() -> Self {
        Self::Bytes { body: Bytes::new() }
    }

    /// Create boxed body from generic message body.
    pub fn new_boxed<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
    {
        Self::Body { body: body.boxed() }
    }

    /// Constructs new `AnyBody` instance from a slice of bytes by copying it.
    ///
    /// If your bytes container is owned, it may be cheaper to use a `From` impl.
    pub fn copy_from_slice(s: &[u8]) -> Self {
        Self::Bytes {
            body: Bytes::copy_from_slice(s),
        }
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Renamed to `copy_from_slice`.")]
    pub fn from_slice(s: &[u8]) -> Self {
        Self::Bytes {
            body: Bytes::copy_from_slice(s),
        }
    }
}

impl<B> AnyBody<B> {
    /// Create body from generic message body.
    pub fn new(body: B) -> Self {
        Self::Body { body }
    }
}

impl<B> AnyBody<B>
where
    B: MessageBody + 'static,
{
    /// Converts a [`MessageBody`] type into the best possible representation.
    ///
    /// Checks size for `None` and tries to convert to `Bytes`. Otherwise, uses the `Body` variant.
    pub fn from_message_body(body: B) -> Self
    where
        B: MessageBody,
    {
        if matches!(body.size(), BodySize::None) {
            return Self::None;
        }

        match body.try_into_bytes() {
            Ok(body) => Self::Bytes { body },
            Err(body) => Self::new(body),
        }
    }

    pub fn into_boxed(self) -> AnyBody {
        match self {
            Self::None => AnyBody::None,
            Self::Bytes { body } => AnyBody::Bytes { body },
            Self::Body { body } => AnyBody::new_boxed(body),
        }
    }
}

impl<B> MessageBody for AnyBody<B>
where
    B: MessageBody,
{
    type Error = crate::BoxError;

    fn size(&self) -> BodySize {
        match self {
            AnyBody::None => BodySize::None,
            AnyBody::Bytes { ref body } => BodySize::Sized(body.len() as u64),
            AnyBody::Body { ref body } => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.project() {
            AnyBodyProj::None => Poll::Ready(None),
            AnyBodyProj::Bytes { body } => {
                let len = body.len();
                if len == 0 {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(mem::take(body))))
                }
            }

            AnyBodyProj::Body { body } => body.poll_next(cx).map_err(|err| err.into()),
        }
    }
}

impl PartialEq for AnyBody {
    fn eq(&self, other: &AnyBody) -> bool {
        match self {
            AnyBody::None => matches!(*other, AnyBody::None),
            AnyBody::Bytes { body } => match other {
                AnyBody::Bytes { body: b2 } => body == b2,
                _ => false,
            },
            AnyBody::Body { .. } => false,
        }
    }
}

impl<S: fmt::Debug> fmt::Debug for AnyBody<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            AnyBody::None => write!(f, "AnyBody::None"),
            AnyBody::Bytes { ref body } => write!(f, "AnyBody::Bytes({:?})", body),
            AnyBody::Body { ref body } => write!(f, "AnyBody::Message({:?})", body),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomPinned;

    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use super::*;

    #[allow(dead_code)]
    struct PinType(PhantomPinned);

    impl MessageBody for PinType {
        type Error = crate::BoxError;

        fn size(&self) -> BodySize {
            unimplemented!()
        }

        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, Self::Error>>> {
            unimplemented!()
        }
    }

    assert_impl_all!(AnyBody<()>: Send, Sync, Unpin, fmt::Debug, MessageBody);
    assert_impl_all!(AnyBody<AnyBody<()>>: Send, Sync, Unpin, fmt::Debug, MessageBody);
    assert_impl_all!(AnyBody<Bytes>: Send, Sync, Unpin, fmt::Debug, MessageBody);
    assert_impl_all!(AnyBody: Unpin, fmt::Debug, MessageBody);
    assert_impl_all!(AnyBody<PinType>: Send, Sync, MessageBody);

    assert_not_impl_any!(AnyBody: Send, Sync);
    assert_not_impl_any!(AnyBody<PinType>: Unpin);
}

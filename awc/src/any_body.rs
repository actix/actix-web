use std::{
    borrow::Cow,
    fmt, mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use pin_project_lite::pin_project;

use actix_http::body::{BodySize, BodyStream, BoxBody, MessageBody, SizedStream};

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
    pub fn into_boxed(self) -> AnyBody {
        match self {
            Self::None => AnyBody::None,
            Self::Bytes { body: bytes } => AnyBody::Bytes { body: bytes },
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

            AnyBodyProj::Body { body } => body
                .poll_next(cx)
                .map_err(|err| format!("{:?}", err).into()),
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

impl<B> From<&'static str> for AnyBody<B> {
    fn from(string: &'static str) -> Self {
        Self::Bytes {
            body: Bytes::from_static(string.as_ref()),
        }
    }
}

impl<B> From<&'static [u8]> for AnyBody<B> {
    fn from(bytes: &'static [u8]) -> Self {
        Self::Bytes {
            body: Bytes::from_static(bytes),
        }
    }
}

impl<B> From<Vec<u8>> for AnyBody<B> {
    fn from(vec: Vec<u8>) -> Self {
        Self::Bytes {
            body: Bytes::from(vec),
        }
    }
}

impl<B> From<String> for AnyBody<B> {
    fn from(string: String) -> Self {
        Self::Bytes {
            body: Bytes::from(string),
        }
    }
}

impl<B> From<&'_ String> for AnyBody<B> {
    fn from(string: &String) -> Self {
        Self::Bytes {
            body: Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(&string)),
        }
    }
}

impl<B> From<Cow<'_, str>> for AnyBody<B> {
    fn from(string: Cow<'_, str>) -> Self {
        match string {
            Cow::Owned(s) => Self::from(s),
            Cow::Borrowed(s) => Self::Bytes {
                body: Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(s)),
            },
        }
    }
}

impl<B> From<Bytes> for AnyBody<B> {
    fn from(bytes: Bytes) -> Self {
        Self::Bytes { body: bytes }
    }
}

impl<B> From<BytesMut> for AnyBody<B> {
    fn from(bytes: BytesMut) -> Self {
        Self::Bytes {
            body: bytes.freeze(),
        }
    }
}

impl<S, E> From<SizedStream<S>> for AnyBody
where
    S: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    fn from(stream: SizedStream<S>) -> Self {
        AnyBody::new_boxed(stream)
    }
}

impl<S, E> From<BodyStream<S>> for AnyBody
where
    S: Stream<Item = Result<Bytes, E>> + 'static,
    E: fmt::Debug + 'static,
{
    fn from(stream: BodyStream<S>) -> Self {
        AnyBody::new_boxed(stream)
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomPinned;

    use static_assertions::{assert_impl_all, assert_not_impl_all};

    use super::*;

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

    assert_impl_all!(AnyBody<()>: MessageBody, fmt::Debug, Send, Sync, Unpin);
    assert_impl_all!(AnyBody<AnyBody<()>>: MessageBody, fmt::Debug, Send, Sync, Unpin);
    assert_impl_all!(AnyBody<Bytes>: MessageBody, fmt::Debug, Send, Sync, Unpin);
    assert_impl_all!(AnyBody: MessageBody, fmt::Debug, Unpin);
    assert_impl_all!(AnyBody<PinType>: MessageBody);

    assert_not_impl_all!(AnyBody: Send, Sync, Unpin);
    assert_not_impl_all!(AnyBody<PinType>: Send, Sync, Unpin);
}

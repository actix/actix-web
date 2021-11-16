use std::{
    borrow::Cow,
    error::Error as StdError,
    fmt, mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_core::Stream;

use crate::error::Error;

use super::{BodySize, BodyStream, MessageBody, MessageBodyMapErr, SizedStream};

pub type Body = AnyBody;

/// Represents various types of HTTP message body.
#[derive(Clone)]
pub enum AnyBody<S = BoxBody> {
    /// Empty response. `Content-Length` header is not set.
    None,

    /// Specific response body.
    Bytes(Bytes),

    /// Generic message body.
    Stream(S),
}

impl AnyBody {
    /// Constructs a new, empty body.
    pub fn empty() -> Self {
        Self::Bytes(Bytes::new())
    }

    /// Create boxed body from generic message body.
    pub fn new_boxed<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
        B::Error: Into<Box<dyn StdError + 'static>>,
    {
        Self::Stream(BoxBody::from_body(body))
    }

    /// Create body from slice (copy)
    pub fn from_slice(s: &[u8]) -> Self {
        Self::Bytes(Bytes::copy_from_slice(s))
    }
}

impl<B> AnyBody<B>
where
    B: MessageBody + 'static,
    B::Error: Into<Box<dyn StdError + 'static>>,
{
    /// Create body from generic message body.
    pub fn new(body: B) -> Self {
        Self::Stream(body)
    }

    pub fn into_boxed(self) -> AnyBody {
        match self {
            AnyBody::None => AnyBody::new_boxed(()),
            AnyBody::Bytes(body) => AnyBody::new_boxed(body),
            AnyBody::Stream(body) => AnyBody::new_boxed(body),
        }
    }
}

impl<S> MessageBody for AnyBody<S>
where
    S: MessageBody + Unpin,
    S::Error: StdError + 'static,
{
    type Error = Error;

    fn size(&self) -> BodySize {
        match self {
            AnyBody::None => BodySize::None,
            AnyBody::Bytes(ref bin) => BodySize::Sized(bin.len() as u64),
            AnyBody::Stream(ref body) => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.get_mut() {
            AnyBody::None => Poll::Ready(None),
            AnyBody::Bytes(ref mut bin) => {
                let len = bin.len();
                if len == 0 {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(mem::take(bin))))
                }
            }

            AnyBody::Stream(body) => Pin::new(body)
                .poll_next(cx)
                .map_err(|err| Error::new_body().with_cause(err)),
        }
    }
}

impl PartialEq for AnyBody {
    fn eq(&self, other: &Body) -> bool {
        match *self {
            AnyBody::None => matches!(*other, AnyBody::None),
            AnyBody::Bytes(ref b) => match *other {
                AnyBody::Bytes(ref b2) => b == b2,
                _ => false,
            },
            AnyBody::Stream(_) => false,
        }
    }
}

impl<S: fmt::Debug> fmt::Debug for AnyBody<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            AnyBody::None => write!(f, "AnyBody::None"),
            AnyBody::Bytes(ref bytes) => write!(f, "AnyBody::Bytes({:?})", bytes),
            AnyBody::Stream(ref stream) => write!(f, "AnyBody::Message({:?})", stream),
        }
    }
}

impl From<&'static str> for AnyBody {
    fn from(string: &'static str) -> Body {
        AnyBody::Bytes(Bytes::from_static(string.as_ref()))
    }
}

impl From<&'static [u8]> for AnyBody {
    fn from(bytes: &'static [u8]) -> Body {
        AnyBody::Bytes(Bytes::from_static(bytes))
    }
}

impl From<Vec<u8>> for AnyBody {
    fn from(vec: Vec<u8>) -> Body {
        AnyBody::Bytes(Bytes::from(vec))
    }
}

impl From<String> for AnyBody {
    fn from(string: String) -> Body {
        string.into_bytes().into()
    }
}

impl From<&'_ String> for AnyBody {
    fn from(string: &String) -> Body {
        AnyBody::Bytes(Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(&string)))
    }
}

impl From<Cow<'_, str>> for AnyBody {
    fn from(string: Cow<'_, str>) -> Body {
        match string {
            Cow::Owned(s) => AnyBody::from(s),
            Cow::Borrowed(s) => {
                AnyBody::Bytes(Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(s)))
            }
        }
    }
}

impl From<Bytes> for AnyBody {
    fn from(bytes: Bytes) -> Body {
        AnyBody::Bytes(bytes)
    }
}

impl From<BytesMut> for AnyBody {
    fn from(bytes: BytesMut) -> Body {
        AnyBody::Bytes(bytes.freeze())
    }
}

impl<S, E> From<SizedStream<S>> for AnyBody
where
    S: Stream<Item = Result<Bytes, E>> + 'static,
    E: StdError + 'static,
{
    fn from(stream: SizedStream<S>) -> Body {
        AnyBody::new_boxed(stream)
    }
}

impl<S, E> From<BodyStream<S>> for AnyBody
where
    S: Stream<Item = Result<Bytes, E>> + 'static,
    E: StdError + 'static,
{
    fn from(stream: BodyStream<S>) -> Body {
        AnyBody::new_boxed(stream)
    }
}

/// A boxed message body with boxed errors.
pub struct BoxBody(Pin<Box<dyn MessageBody<Error = Box<dyn StdError>>>>);

impl BoxBody {
    /// Boxes a `MessageBody` and any errors it generates.
    pub fn from_body<B>(body: B) -> Self
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

    assert_impl_all!(AnyBody<()>: MessageBody, fmt::Debug, Send, Sync);
    assert_impl_all!(AnyBody<Bytes>: MessageBody, fmt::Debug, Send, Sync);
    assert_impl_all!(AnyBody: MessageBody, fmt::Debug);
    assert_impl_all!(BoxBody: MessageBody, fmt::Debug);

    assert_not_impl_all!(AnyBody: Send, Sync);
    assert_not_impl_all!(BoxBody: Send, Sync);
}

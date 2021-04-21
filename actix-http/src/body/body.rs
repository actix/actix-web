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

/// Represents various types of HTTP message body.
// #[deprecated(since = "4.0.0", note = "Use body types directly.")]
pub enum Body {
    /// Empty response. `Content-Length` header is not set.
    None,

    /// Zero sized response body. `Content-Length` header is set to `0`.
    Empty,

    /// Specific response body.
    Bytes(Bytes),

    /// Generic message body.
    Message(BoxAnyBody),
}

impl Body {
    /// Create body from slice (copy)
    pub fn from_slice(s: &[u8]) -> Body {
        Body::Bytes(Bytes::copy_from_slice(s))
    }

    /// Create body from generic message body.
    pub fn from_message<B>(body: B) -> Body
    where
        B: MessageBody + 'static,
        B::Error: Into<Box<dyn StdError + 'static>>,
    {
        Self::Message(BoxAnyBody::from_body(body))
    }
}

impl MessageBody for Body {
    type Error = Error;

    fn size(&self) -> BodySize {
        match self {
            Body::None => BodySize::None,
            Body::Empty => BodySize::Empty,
            Body::Bytes(ref bin) => BodySize::Sized(bin.len() as u64),
            Body::Message(ref body) => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        match self.get_mut() {
            Body::None => Poll::Ready(None),
            Body::Empty => Poll::Ready(None),
            Body::Bytes(ref mut bin) => {
                let len = bin.len();
                if len == 0 {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(mem::take(bin))))
                }
            }
            Body::Message(body) => body.as_mut().poll_next(cx).map_err(Into::into),
        }
    }
}

impl PartialEq for Body {
    fn eq(&self, other: &Body) -> bool {
        match *self {
            Body::None => matches!(*other, Body::None),
            Body::Empty => matches!(*other, Body::Empty),
            Body::Bytes(ref b) => match *other {
                Body::Bytes(ref b2) => b == b2,
                _ => false,
            },
            Body::Message(_) => false,
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Body::None => write!(f, "Body::None"),
            Body::Empty => write!(f, "Body::Empty"),
            Body::Bytes(ref b) => write!(f, "Body::Bytes({:?})", b),
            Body::Message(_) => write!(f, "Body::Message(_)"),
        }
    }
}

impl From<&'static str> for Body {
    fn from(s: &'static str) -> Body {
        Body::Bytes(Bytes::from_static(s.as_ref()))
    }
}

impl From<&'static [u8]> for Body {
    fn from(s: &'static [u8]) -> Body {
        Body::Bytes(Bytes::from_static(s))
    }
}

impl From<Vec<u8>> for Body {
    fn from(vec: Vec<u8>) -> Body {
        Body::Bytes(Bytes::from(vec))
    }
}

impl From<String> for Body {
    fn from(s: String) -> Body {
        s.into_bytes().into()
    }
}

impl From<&'_ String> for Body {
    fn from(s: &String) -> Body {
        Body::Bytes(Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(&s)))
    }
}

impl From<Cow<'_, str>> for Body {
    fn from(s: Cow<'_, str>) -> Body {
        match s {
            Cow::Owned(s) => Body::from(s),
            Cow::Borrowed(s) => {
                Body::Bytes(Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(s)))
            }
        }
    }
}

impl From<Bytes> for Body {
    fn from(s: Bytes) -> Body {
        Body::Bytes(s)
    }
}

impl From<BytesMut> for Body {
    fn from(s: BytesMut) -> Body {
        Body::Bytes(s.freeze())
    }
}

impl<S> From<SizedStream<S>> for Body
where
    S: Stream<Item = Result<Bytes, Error>> + 'static,
{
    fn from(s: SizedStream<S>) -> Body {
        Body::from_message(s)
    }
}

impl<S, E> From<BodyStream<S>> for Body
where
    S: Stream<Item = Result<Bytes, E>> + 'static,
    E: Into<Error> + 'static,
{
    fn from(s: BodyStream<S>) -> Body {
        Body::from_message(s)
    }
}

/// A boxed message body with boxed errors.
pub struct BoxAnyBody(Pin<Box<dyn MessageBody<Error = Box<dyn StdError + 'static>>>>);

impl BoxAnyBody {
    pub fn from_body<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
        B::Error: Into<Box<dyn StdError + 'static>>,
    {
        let body = MessageBodyMapErr::new(body, Into::into);
        Self(Box::pin(body))
    }

    /// Returns a mutable pinned reference to the inner message body type.
    pub fn as_mut(
        &mut self,
    ) -> Pin<&mut (dyn MessageBody<Error = Box<dyn StdError + 'static>>)> {
        self.0.as_mut()
    }
}

impl fmt::Debug for BoxAnyBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BoxAnyBody(dyn MessageBody)")
    }
}

impl MessageBody for BoxAnyBody {
    type Error = Error;

    fn size(&self) -> BodySize {
        self.0.size()
    }

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        self.0.as_mut().poll_next(cx).map_err(Into::into)
    }
}

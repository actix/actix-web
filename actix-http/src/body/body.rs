use std::{
    borrow::Cow,
    error::Error as StdError,
    fmt, mem,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use futures_core::{ready, Stream};

use crate::error::Error;

use super::{BodySize, BodyStream, MessageBody, MessageBodyMapErr, SizedStream};

pub type Body = AnyBody;

/// Represents various types of HTTP message body.
pub enum AnyBody {
    /// Empty response. `Content-Length` header is not set.
    None,

    /// Zero sized response body. `Content-Length` header is set to `0`.
    Empty,

    /// Specific response body.
    Bytes(Bytes),

    /// Generic message body.
    Message(BoxAnyBody),
}

impl AnyBody {
    /// Create body from slice (copy)
    pub fn from_slice(s: &[u8]) -> Self {
        Self::Bytes(Bytes::copy_from_slice(s))
    }

    /// Create body from generic message body.
    pub fn from_message<B>(body: B) -> Self
    where
        B: MessageBody + 'static,
        B::Error: Into<Box<dyn StdError + 'static>>,
    {
        Self::Message(BoxAnyBody::from_body(body))
    }
}

impl MessageBody for AnyBody {
    type Error = Error;

    fn size(&self) -> BodySize {
        match self {
            AnyBody::None => BodySize::None,
            AnyBody::Empty => BodySize::Empty,
            AnyBody::Bytes(ref bin) => BodySize::Sized(bin.len() as u64),
            AnyBody::Message(ref body) => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.get_mut() {
            AnyBody::None => Poll::Ready(None),
            AnyBody::Empty => Poll::Ready(None),
            AnyBody::Bytes(ref mut bin) => {
                let len = bin.len();
                if len == 0 {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(mem::take(bin))))
                }
            }

            // TODO: MSRV 1.51: poll_map_err
            AnyBody::Message(body) => match ready!(body.as_pin_mut().poll_next(cx)) {
                Some(Err(err)) => {
                    Poll::Ready(Some(Err(Error::new_body().with_cause(err))))
                }
                Some(Ok(val)) => Poll::Ready(Some(Ok(val))),
                None => Poll::Ready(None),
            },
        }
    }
}

impl PartialEq for AnyBody {
    fn eq(&self, other: &Body) -> bool {
        match *self {
            AnyBody::None => matches!(*other, AnyBody::None),
            AnyBody::Empty => matches!(*other, AnyBody::Empty),
            AnyBody::Bytes(ref b) => match *other {
                AnyBody::Bytes(ref b2) => b == b2,
                _ => false,
            },
            AnyBody::Message(_) => false,
        }
    }
}

impl fmt::Debug for AnyBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            AnyBody::None => write!(f, "AnyBody::None"),
            AnyBody::Empty => write!(f, "AnyBody::Empty"),
            AnyBody::Bytes(ref b) => write!(f, "AnyBody::Bytes({:?})", b),
            AnyBody::Message(_) => write!(f, "AnyBody::Message(_)"),
        }
    }
}

impl From<&'static str> for AnyBody {
    fn from(s: &'static str) -> Body {
        AnyBody::Bytes(Bytes::from_static(s.as_ref()))
    }
}

impl From<&'static [u8]> for AnyBody {
    fn from(s: &'static [u8]) -> Body {
        AnyBody::Bytes(Bytes::from_static(s))
    }
}

impl From<Vec<u8>> for AnyBody {
    fn from(vec: Vec<u8>) -> Body {
        AnyBody::Bytes(Bytes::from(vec))
    }
}

impl From<String> for AnyBody {
    fn from(s: String) -> Body {
        s.into_bytes().into()
    }
}

impl From<&'_ String> for AnyBody {
    fn from(s: &String) -> Body {
        AnyBody::Bytes(Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(&s)))
    }
}

impl From<Cow<'_, str>> for AnyBody {
    fn from(s: Cow<'_, str>) -> Body {
        match s {
            Cow::Owned(s) => AnyBody::from(s),
            Cow::Borrowed(s) => {
                AnyBody::Bytes(Bytes::copy_from_slice(AsRef::<[u8]>::as_ref(s)))
            }
        }
    }
}

impl From<Bytes> for AnyBody {
    fn from(s: Bytes) -> Body {
        AnyBody::Bytes(s)
    }
}

impl From<BytesMut> for AnyBody {
    fn from(s: BytesMut) -> Body {
        AnyBody::Bytes(s.freeze())
    }
}

impl<S, E> From<SizedStream<S>> for AnyBody
where
    S: Stream<Item = Result<Bytes, E>> + 'static,
    E: Into<Box<dyn StdError>> + 'static,
{
    fn from(s: SizedStream<S>) -> Body {
        AnyBody::from_message(s)
    }
}

impl<S, E> From<BodyStream<S>> for AnyBody
where
    S: Stream<Item = Result<Bytes, E>> + 'static,
    E: Into<Box<dyn StdError>> + 'static,
{
    fn from(s: BodyStream<S>) -> Body {
        AnyBody::from_message(s)
    }
}

/// A boxed message body with boxed errors.
pub struct BoxAnyBody(Pin<Box<dyn MessageBody<Error = Box<dyn StdError + 'static>>>>);

impl BoxAnyBody {
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
        // TODO: MSRV 1.51: poll_map_err
        match ready!(self.0.as_mut().poll_next(cx)) {
            Some(Err(err)) => Poll::Ready(Some(Err(Error::new_body().with_cause(err)))),
            Some(Ok(val)) => Poll::Ready(Some(Ok(val))),
            None => Poll::Ready(None),
        }
    }
}

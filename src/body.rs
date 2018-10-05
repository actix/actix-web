use bytes::{Bytes, BytesMut};
use futures::Stream;
use std::sync::Arc;
use std::{fmt, mem};

use error::Error;
use httpresponse::HttpResponse;

/// Type represent streaming body
pub type BodyStream = Box<Stream<Item = Bytes, Error = Error>>;

/// Represents various types of http message body.
pub enum Body {
    /// Empty response. `Content-Length` header is set to `0`
    Empty,
    /// Specific response body.
    Binary(Binary),
    /// Unspecified streaming response. Developer is responsible for setting
    /// right `Content-Length` or `Transfer-Encoding` headers.
    Streaming(BodyStream),
    // /// Special body type for actor response.
    // Actor(Box<ActorHttpContext>),
}

/// Represents various types of binary body.
/// `Content-Length` header is set to length of the body.
#[derive(Debug, PartialEq)]
pub enum Binary {
    /// Bytes body
    Bytes(Bytes),
    /// Static slice
    Slice(&'static [u8]),
    /// Shared string body
    #[doc(hidden)]
    SharedString(Arc<String>),
    /// Shared vec body
    SharedVec(Arc<Vec<u8>>),
}

impl Body {
    /// Does this body streaming.
    #[inline]
    pub fn is_streaming(&self) -> bool {
        match *self {
            Body::Streaming(_) => true,
            _ => false,
        }
    }

    /// Is this binary body.
    #[inline]
    pub fn is_binary(&self) -> bool {
        match *self {
            Body::Binary(_) => true,
            _ => false,
        }
    }

    /// Is this binary empy.
    #[inline]
    pub fn is_empty(&self) -> bool {
        match *self {
            Body::Empty => true,
            _ => false,
        }
    }

    /// Create body from slice (copy)
    pub fn from_slice(s: &[u8]) -> Body {
        Body::Binary(Binary::Bytes(Bytes::from(s)))
    }

    /// Is this binary body.
    #[inline]
    pub(crate) fn binary(self) -> Binary {
        match self {
            Body::Binary(b) => b,
            _ => panic!(),
        }
    }
}

impl PartialEq for Body {
    fn eq(&self, other: &Body) -> bool {
        match *self {
            Body::Empty => match *other {
                Body::Empty => true,
                _ => false,
            },
            Body::Binary(ref b) => match *other {
                Body::Binary(ref b2) => b == b2,
                _ => false,
            },
            Body::Streaming(_) => false,
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Body::Empty => write!(f, "Body::Empty"),
            Body::Binary(ref b) => write!(f, "Body::Binary({:?})", b),
            Body::Streaming(_) => write!(f, "Body::Streaming(_)"),
        }
    }
}

impl<T> From<T> for Body
where
    T: Into<Binary>,
{
    fn from(b: T) -> Body {
        Body::Binary(b.into())
    }
}

impl Binary {
    #[inline]
    /// Returns `true` if body is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    /// Length of body in bytes
    pub fn len(&self) -> usize {
        match *self {
            Binary::Bytes(ref bytes) => bytes.len(),
            Binary::Slice(slice) => slice.len(),
            Binary::SharedString(ref s) => s.len(),
            Binary::SharedVec(ref s) => s.len(),
        }
    }

    /// Create binary body from slice
    pub fn from_slice(s: &[u8]) -> Binary {
        Binary::Bytes(Bytes::from(s))
    }

    /// Convert Binary to a Bytes instance
    pub fn take(&mut self) -> Bytes {
        mem::replace(self, Binary::Slice(b"")).into()
    }
}

impl Clone for Binary {
    fn clone(&self) -> Binary {
        match *self {
            Binary::Bytes(ref bytes) => Binary::Bytes(bytes.clone()),
            Binary::Slice(slice) => Binary::Bytes(Bytes::from(slice)),
            Binary::SharedString(ref s) => Binary::SharedString(s.clone()),
            Binary::SharedVec(ref s) => Binary::SharedVec(s.clone()),
        }
    }
}

impl Into<Bytes> for Binary {
    fn into(self) -> Bytes {
        match self {
            Binary::Bytes(bytes) => bytes,
            Binary::Slice(slice) => Bytes::from(slice),
            Binary::SharedString(s) => Bytes::from(s.as_str()),
            Binary::SharedVec(s) => Bytes::from(AsRef::<[u8]>::as_ref(s.as_ref())),
        }
    }
}

impl From<&'static str> for Binary {
    fn from(s: &'static str) -> Binary {
        Binary::Slice(s.as_ref())
    }
}

impl From<&'static [u8]> for Binary {
    fn from(s: &'static [u8]) -> Binary {
        Binary::Slice(s)
    }
}

impl From<Vec<u8>> for Binary {
    fn from(vec: Vec<u8>) -> Binary {
        Binary::Bytes(Bytes::from(vec))
    }
}

impl From<String> for Binary {
    fn from(s: String) -> Binary {
        Binary::Bytes(Bytes::from(s))
    }
}

impl<'a> From<&'a String> for Binary {
    fn from(s: &'a String) -> Binary {
        Binary::Bytes(Bytes::from(AsRef::<[u8]>::as_ref(&s)))
    }
}

impl From<Bytes> for Binary {
    fn from(s: Bytes) -> Binary {
        Binary::Bytes(s)
    }
}

impl From<BytesMut> for Binary {
    fn from(s: BytesMut) -> Binary {
        Binary::Bytes(s.freeze())
    }
}

impl From<Arc<String>> for Binary {
    fn from(body: Arc<String>) -> Binary {
        Binary::SharedString(body)
    }
}

impl<'a> From<&'a Arc<String>> for Binary {
    fn from(body: &'a Arc<String>) -> Binary {
        Binary::SharedString(Arc::clone(body))
    }
}

impl From<Arc<Vec<u8>>> for Binary {
    fn from(body: Arc<Vec<u8>>) -> Binary {
        Binary::SharedVec(body)
    }
}

impl<'a> From<&'a Arc<Vec<u8>>> for Binary {
    fn from(body: &'a Arc<Vec<u8>>) -> Binary {
        Binary::SharedVec(Arc::clone(body))
    }
}

impl AsRef<[u8]> for Binary {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        match *self {
            Binary::Bytes(ref bytes) => bytes.as_ref(),
            Binary::Slice(slice) => slice,
            Binary::SharedString(ref s) => s.as_bytes(),
            Binary::SharedVec(ref s) => s.as_ref().as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_body_is_streaming() {
        assert_eq!(Body::Empty.is_streaming(), false);
        assert_eq!(Body::Binary(Binary::from("")).is_streaming(), false);
    }

    #[test]
    fn test_is_empty() {
        assert_eq!(Binary::from("").is_empty(), true);
        assert_eq!(Binary::from("test").is_empty(), false);
    }

    #[test]
    fn test_static_str() {
        assert_eq!(Binary::from("test").len(), 4);
        assert_eq!(Binary::from("test").as_ref(), b"test");
    }

    #[test]
    fn test_static_bytes() {
        assert_eq!(Binary::from(b"test".as_ref()).len(), 4);
        assert_eq!(Binary::from(b"test".as_ref()).as_ref(), b"test");
        assert_eq!(Binary::from_slice(b"test".as_ref()).len(), 4);
        assert_eq!(Binary::from_slice(b"test".as_ref()).as_ref(), b"test");
    }

    #[test]
    fn test_vec() {
        assert_eq!(Binary::from(Vec::from("test")).len(), 4);
        assert_eq!(Binary::from(Vec::from("test")).as_ref(), b"test");
    }

    #[test]
    fn test_bytes() {
        assert_eq!(Binary::from(Bytes::from("test")).len(), 4);
        assert_eq!(Binary::from(Bytes::from("test")).as_ref(), b"test");
    }

    #[test]
    fn test_arc_string() {
        let b = Arc::new("test".to_owned());
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), b"test");
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), b"test");
    }

    #[test]
    fn test_string() {
        let b = "test".to_owned();
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), b"test");
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), b"test");
    }

    #[test]
    fn test_shared_vec() {
        let b = Arc::new(Vec::from(&b"test"[..]));
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), &b"test"[..]);
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), &b"test"[..]);
    }

    #[test]
    fn test_bytes_mut() {
        let b = BytesMut::from("test");
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b).as_ref(), b"test");
    }

    #[test]
    fn test_binary_into() {
        let bytes = Bytes::from_static(b"test");
        let b: Bytes = Binary::from("test").into();
        assert_eq!(b, bytes);
        let b: Bytes = Binary::from(bytes.clone()).into();
        assert_eq!(b, bytes);
    }
}

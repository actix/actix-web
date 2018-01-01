use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use bytes::{Bytes, BytesMut};
use futures::Stream;

use error::Error;
use context::ActorHttpContext;

/// Type represent streaming body
pub type BodyStream = Box<Stream<Item=Bytes, Error=Error>>;

/// Represents various types of http message body.
pub enum Body {
    /// Empty response. `Content-Length` header is set to `0`
    Empty,
    /// Specific response body.
    Binary(Binary),
    /// Unspecified streaming response. Developer is responsible for setting
    /// right `Content-Length` or `Transfer-Encoding` headers.
    Streaming(BodyStream),
    /// Special body type for actor response.
    Actor(Box<ActorHttpContext>),
}

/// Represents various types of binary body.
/// `Content-Length` header is set to length of the body.
#[derive(Debug, PartialEq)]
pub enum Binary {
    /// Bytes body
    Bytes(Bytes),
    /// Static slice
    Slice(&'static [u8]),
    /// Shared bytes body
    SharedBytes(Rc<Bytes>),
    /// Shared stirng body
    SharedString(Rc<String>),
    /// Shared bytes body
    #[doc(hidden)]
    ArcSharedBytes(Arc<Bytes>),
    /// Shared string body
    #[doc(hidden)]
    ArcSharedString(Arc<String>),
}

impl Body {
    /// Does this body streaming.
    #[inline]
    pub fn is_streaming(&self) -> bool {
        match *self {
            Body::Streaming(_) | Body::Actor(_) => true,
            _ => false
        }
    }

    /// Is this binary body.
    #[inline]
    pub fn is_binary(&self) -> bool {
        match *self {
            Body::Binary(_) => true,
            _ => false
        }
    }

    /// Create body from slice (copy)
    pub fn from_slice(s: &[u8]) -> Body {
        Body::Binary(Binary::Bytes(Bytes::from(s)))
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
            Body::Streaming(_) | Body::Actor(_) => false,
        }
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Body::Empty => write!(f, "Body::Empty"),
            Body::Binary(ref b) => write!(f, "Body::Binary({:?})", b),
            Body::Streaming(_) => write!(f, "Body::Streaming(_)"),
            Body::Actor(_) => write!(f, "Body::Actor(_)"),
        }
    }
}

impl<T> From<T> for Body where T: Into<Binary>{
    fn from(b: T) -> Body {
        Body::Binary(b.into())
    }
}

impl From<Box<ActorHttpContext>> for Body {
    fn from(ctx: Box<ActorHttpContext>) -> Body {
        Body::Actor(ctx)
    }
}

impl Binary {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn len(&self) -> usize {
        match *self {
            Binary::Bytes(ref bytes) => bytes.len(),
            Binary::Slice(slice) => slice.len(),
            Binary::SharedBytes(ref bytes) => bytes.len(),
            Binary::ArcSharedBytes(ref bytes) => bytes.len(),
            Binary::SharedString(ref s) => s.len(),
            Binary::ArcSharedString(ref s) => s.len(),
        }
    }

    /// Create binary body from slice
    pub fn from_slice(s: &[u8]) -> Binary {
        Binary::Bytes(Bytes::from(s))
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

impl From<Rc<Bytes>> for Binary {
    fn from(body: Rc<Bytes>) -> Binary {
        Binary::SharedBytes(body)
    }
}

impl<'a> From<&'a Rc<Bytes>> for Binary {
    fn from(body: &'a Rc<Bytes>) -> Binary {
        Binary::SharedBytes(Rc::clone(body))
    }
}

impl From<Arc<Bytes>> for Binary {
    fn from(body: Arc<Bytes>) -> Binary {
        Binary::ArcSharedBytes(body)
    }
}

impl<'a> From<&'a Arc<Bytes>> for Binary {
    fn from(body: &'a Arc<Bytes>) -> Binary {
        Binary::ArcSharedBytes(Arc::clone(body))
    }
}

impl From<Rc<String>> for Binary {
    fn from(body: Rc<String>) -> Binary {
        Binary::SharedString(body)
    }
}

impl<'a> From<&'a Rc<String>> for Binary {
    fn from(body: &'a Rc<String>) -> Binary {
        Binary::SharedString(Rc::clone(body))
    }
}

impl From<Arc<String>> for Binary {
    fn from(body: Arc<String>) -> Binary {
        Binary::ArcSharedString(body)
    }
}

impl<'a> From<&'a Arc<String>> for Binary {
    fn from(body: &'a Arc<String>) -> Binary {
        Binary::ArcSharedString(Arc::clone(body))
    }
}

impl AsRef<[u8]> for Binary {
    fn as_ref(&self) -> &[u8] {
        match *self {
            Binary::Bytes(ref bytes) => bytes.as_ref(),
            Binary::Slice(slice) => slice,
            Binary::SharedBytes(ref bytes) => bytes.as_ref(),
            Binary::ArcSharedBytes(ref bytes) => bytes.as_ref(),
            Binary::SharedString(ref s) => s.as_bytes(),
            Binary::ArcSharedString(ref s) => s.as_bytes(),
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
        // assert_eq!(Body::Streaming.is_streaming(), true);
    }

    #[test]
    fn test_is_empty() {
        assert_eq!(Binary::from("").is_empty(), true);
        assert_eq!(Binary::from("test").is_empty(), false);
    }

    #[test]
    fn test_static_str() {
        assert_eq!(Binary::from("test").len(), 4);
        assert_eq!(Binary::from("test").as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_static_bytes() {
        assert_eq!(Binary::from(b"test".as_ref()).len(), 4);
        assert_eq!(Binary::from(b"test".as_ref()).as_ref(), "test".as_bytes());
        assert_eq!(Binary::from_slice(b"test".as_ref()).len(), 4);
        assert_eq!(Binary::from_slice(b"test".as_ref()).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_vec() {
        assert_eq!(Binary::from(Vec::from("test")).len(), 4);
        assert_eq!(Binary::from(Vec::from("test")).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_bytes() {
        assert_eq!(Binary::from(Bytes::from("test")).len(), 4);
        assert_eq!(Binary::from(Bytes::from("test")).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_rc_bytes() {
        let b = Rc::new(Bytes::from("test"));
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), "test".as_bytes());
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_ref_string() {
        let b = Rc::new("test".to_owned());
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_rc_string() {
        let b = Rc::new("test".to_owned());
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), "test".as_bytes());
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_arc_bytes() {
        let b = Arc::new(Bytes::from("test"));
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), "test".as_bytes());
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_arc_string() {
        let b = Arc::new("test".to_owned());
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), "test".as_bytes());
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_string() {
        let b = "test".to_owned();
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b.clone()).as_ref(), "test".as_bytes());
        assert_eq!(Binary::from(&b).len(), 4);
        assert_eq!(Binary::from(&b).as_ref(), "test".as_bytes());
    }

    #[test]
    fn test_bytes_mut() {
        let b =  BytesMut::from("test");
        assert_eq!(Binary::from(b.clone()).len(), 4);
        assert_eq!(Binary::from(b).as_ref(), "test".as_bytes());
    }
}

use std::rc::Rc;
use std::sync::Arc;
use bytes::{Bytes, BytesMut};


/// Represents various types of http message body.
#[derive(Debug)]
pub enum Body {
    /// Empty response. `Content-Length` header is set to `0`
    Empty,
    /// Specific response body.
    Binary(BinaryBody),
    /// Streaming response body with specified length.
    Length(u64),
    /// Unspecified streaming response. Developer is responsible for setting
    /// right `Content-Length` or `Transfer-Encoding` headers.
    Streaming,
    /// Upgrade connection.
    Upgrade,
}

/// Represents various types of binary body.
/// `Content-Length` header is set to length of the body.
#[derive(Debug)]
pub enum BinaryBody {
    /// Bytes body
    Bytes(Bytes),
    /// Static slice
    Slice(&'static [u8]),
    /// Shared bytes body
    SharedBytes(Rc<Bytes>),
    /// Shared bytes body
    #[doc(hidden)]
    ArcSharedBytes(Arc<Bytes>),
}

impl Body {
    /// Does this body have payload.
    pub fn has_body(&self) -> bool {
        match *self {
            Body::Length(_) | Body::Streaming => true,
            _ => false
        }
    }

    /// Create body from slice (copy)
    pub fn from_slice<'a>(s: &'a [u8]) -> Body {
        Body::Binary(BinaryBody::Bytes(Bytes::from(s)))
    }
}

impl<T> From<T> for Body where T: Into<BinaryBody>{
    fn from(b: T) -> Body {
        Body::Binary(b.into())
    }
}

impl BinaryBody {
    pub fn len(&self) -> usize {
        match self {
            &BinaryBody::Bytes(ref bytes) => bytes.len(),
            &BinaryBody::Slice(slice) => slice.len(),
            &BinaryBody::SharedBytes(ref bytes) => bytes.len(),
            &BinaryBody::ArcSharedBytes(ref bytes) => bytes.len(),
        }
    }

    /// Create binary body from slice
    pub fn from_slice<'a>(s: &'a [u8]) -> BinaryBody {
        BinaryBody::Bytes(Bytes::from(s))
    }
}

impl From<&'static str> for BinaryBody {
    fn from(s: &'static str) -> BinaryBody {
        BinaryBody::Slice(s.as_ref())
    }
}

impl From<&'static [u8]> for BinaryBody {
    fn from(s: &'static [u8]) -> BinaryBody {
        BinaryBody::Slice(s)
    }
}

impl From<Vec<u8>> for BinaryBody {
    fn from(vec: Vec<u8>) -> BinaryBody {
        BinaryBody::Bytes(Bytes::from(vec))
    }
}

impl From<String> for BinaryBody {
    fn from(s: String) -> BinaryBody {
        BinaryBody::Bytes(Bytes::from(s))
    }
}

impl From<Bytes> for BinaryBody {
    fn from(s: Bytes) -> BinaryBody {
        BinaryBody::Bytes(s)
    }
}

impl From<BytesMut> for BinaryBody {
    fn from(s: BytesMut) -> BinaryBody {
        BinaryBody::Bytes(s.freeze())
    }
}

impl From<Rc<Bytes>> for BinaryBody {
    fn from(body: Rc<Bytes>) -> BinaryBody {
        BinaryBody::SharedBytes(body)
    }
}

impl<'a> From<&'a Rc<Bytes>> for BinaryBody {
    fn from(body: &'a Rc<Bytes>) -> BinaryBody {
        BinaryBody::SharedBytes(Rc::clone(body))
    }
}

impl From<Arc<Bytes>> for BinaryBody {
    fn from(body: Arc<Bytes>) -> BinaryBody {
        BinaryBody::ArcSharedBytes(body)
    }
}

impl<'a> From<&'a Arc<Bytes>> for BinaryBody {
    fn from(body: &'a Arc<Bytes>) -> BinaryBody {
        BinaryBody::ArcSharedBytes(Arc::clone(body))
    }
}

impl AsRef<[u8]> for BinaryBody {
    fn as_ref(&self) -> &[u8] {
        match self {
            &BinaryBody::Bytes(ref bytes) => bytes.as_ref(),
            &BinaryBody::Slice(slice) => slice,
            &BinaryBody::SharedBytes(ref bytes) => bytes.as_ref(),
            &BinaryBody::ArcSharedBytes(ref bytes) => bytes.as_ref(),
        }
    }
}

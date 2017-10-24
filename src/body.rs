use std::rc::Rc;
use std::sync::Arc;
use bytes::Bytes;


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

    /// Create body from static string
    pub fn from_slice<'a>(s: &'a [u8]) -> Body {
        Body::Binary(BinaryBody::Bytes(Bytes::from(s)))
    }
}

impl From<&'static str> for Body {
    fn from(s: &'static str) -> Body {
        Body::Binary(BinaryBody::Slice(s.as_ref()))
    }
}

impl From<&'static [u8]> for Body {
    fn from(s: &'static [u8]) -> Body {
        Body::Binary(BinaryBody::Slice(s))
    }
}

impl From<Vec<u8>> for Body {
    fn from(vec: Vec<u8>) -> Body {
        Body::Binary(BinaryBody::Bytes(Bytes::from(vec)))
    }
}

impl From<String> for Body {
    fn from(s: String) -> Body {
        Body::Binary(BinaryBody::Bytes(Bytes::from(s)))
    }
}

impl From<Rc<Bytes>> for Body {
    fn from(body: Rc<Bytes>) -> Body {
        Body::Binary(BinaryBody::SharedBytes(body))
    }
}

impl From<Arc<Bytes>> for Body {
    fn from(body: Arc<Bytes>) -> Body {
        Body::Binary(BinaryBody::ArcSharedBytes(body))
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

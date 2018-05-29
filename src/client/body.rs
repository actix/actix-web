use std::fmt;

use bytes::Bytes;
use futures::Stream;

use body::Binary;
use context::ActorHttpContext;
use error::Error;

/// Type represent streaming body
pub type ClientBodyStream = Box<Stream<Item = Bytes, Error = Error> + Send>;

/// Represents various types of http message body.
pub enum ClientBody {
    /// Empty response. `Content-Length` header is set to `0`
    Empty,
    /// Specific response body.
    Binary(Binary),
    /// Unspecified streaming response. Developer is responsible for setting
    /// right `Content-Length` or `Transfer-Encoding` headers.
    Streaming(ClientBodyStream),
    /// Special body type for actor response.
    Actor(Box<ActorHttpContext + Send>),
}

impl ClientBody {
    /// Does this body streaming.
    #[inline]
    pub fn is_streaming(&self) -> bool {
        match *self {
            ClientBody::Streaming(_) | ClientBody::Actor(_) => true,
            _ => false,
        }
    }

    /// Is this binary body.
    #[inline]
    pub fn is_binary(&self) -> bool {
        match *self {
            ClientBody::Binary(_) => true,
            _ => false,
        }
    }

    /// Is this binary empy.
    #[inline]
    pub fn is_empty(&self) -> bool {
        match *self {
            ClientBody::Empty => true,
            _ => false,
        }
    }

    /// Create body from slice (copy)
    pub fn from_slice(s: &[u8]) -> ClientBody {
        ClientBody::Binary(Binary::Bytes(Bytes::from(s)))
    }
}

impl PartialEq for ClientBody {
    fn eq(&self, other: &ClientBody) -> bool {
        match *self {
            ClientBody::Empty => match *other {
                ClientBody::Empty => true,
                _ => false,
            },
            ClientBody::Binary(ref b) => match *other {
                ClientBody::Binary(ref b2) => b == b2,
                _ => false,
            },
            ClientBody::Streaming(_) | ClientBody::Actor(_) => false,
        }
    }
}

impl fmt::Debug for ClientBody {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ClientBody::Empty => write!(f, "ClientBody::Empty"),
            ClientBody::Binary(ref b) => write!(f, "ClientBody::Binary({:?})", b),
            ClientBody::Streaming(_) => write!(f, "ClientBody::Streaming(_)"),
            ClientBody::Actor(_) => write!(f, "ClientBody::Actor(_)"),
        }
    }
}

impl<T> From<T> for ClientBody
where
    T: Into<Binary>,
{
    fn from(b: T) -> ClientBody {
        ClientBody::Binary(b.into())
    }
}

use std::{net, rc::Rc};

use crate::{
    header::{self, HeaderMap},
    message::{Flags, Head, MessagePool},
    ConnectionType, Method, Uri, Version,
};

thread_local! {
    static REQUEST_POOL: MessagePool<RequestHead> = MessagePool::<RequestHead>::create()
}

#[derive(Debug, Clone)]
pub struct RequestHead {
    pub method: Method,
    pub uri: Uri,
    pub version: Version,
    pub headers: HeaderMap,

    /// Will only be None when called in unit tests unless set manually.
    pub peer_addr: Option<net::SocketAddr>,

    flags: Flags,
}

impl Default for RequestHead {
    fn default() -> RequestHead {
        RequestHead {
            method: Method::default(),
            uri: Uri::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            peer_addr: None,
            flags: Flags::empty(),
        }
    }
}

impl Head for RequestHead {
    fn clear(&mut self) {
        self.flags = Flags::empty();
        self.headers.clear();
    }

    fn with_pool<F, R>(f: F) -> R
    where
        F: FnOnce(&MessagePool<Self>) -> R,
    {
        REQUEST_POOL.with(|p| f(p))
    }
}

impl RequestHead {
    /// Read the message headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Mutable reference to the message headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Is to uppercase headers with Camel-Case.
    /// Default is `false`
    #[inline]
    pub fn camel_case_headers(&self) -> bool {
        self.flags.contains(Flags::CAMEL_CASE)
    }

    /// Set `true` to send headers which are formatted as Camel-Case.
    #[inline]
    pub fn set_camel_case_headers(&mut self, val: bool) {
        if val {
            self.flags.insert(Flags::CAMEL_CASE);
        } else {
            self.flags.remove(Flags::CAMEL_CASE);
        }
    }

    #[inline]
    /// Set connection type of the message
    pub fn set_connection_type(&mut self, ctype: ConnectionType) {
        match ctype {
            ConnectionType::Close => self.flags.insert(Flags::CLOSE),
            ConnectionType::KeepAlive => self.flags.insert(Flags::KEEP_ALIVE),
            ConnectionType::Upgrade => self.flags.insert(Flags::UPGRADE),
        }
    }

    #[inline]
    /// Connection type
    pub fn connection_type(&self) -> ConnectionType {
        if self.flags.contains(Flags::CLOSE) {
            ConnectionType::Close
        } else if self.flags.contains(Flags::KEEP_ALIVE) {
            ConnectionType::KeepAlive
        } else if self.flags.contains(Flags::UPGRADE) {
            ConnectionType::Upgrade
        } else if self.version < Version::HTTP_11 {
            ConnectionType::Close
        } else {
            ConnectionType::KeepAlive
        }
    }

    /// Connection upgrade status
    pub fn upgrade(&self) -> bool {
        self.headers()
            .get(header::CONNECTION)
            .map(|hdr| {
                if let Ok(s) = hdr.to_str() {
                    s.to_ascii_lowercase().contains("upgrade")
                } else {
                    false
                }
            })
            .unwrap_or(false)
    }

    #[inline]
    /// Get response body chunking state
    pub fn chunked(&self) -> bool {
        !self.flags.contains(Flags::NO_CHUNKING)
    }

    #[inline]
    pub fn no_chunking(&mut self, val: bool) {
        if val {
            self.flags.insert(Flags::NO_CHUNKING);
        } else {
            self.flags.remove(Flags::NO_CHUNKING);
        }
    }

    /// Request contains `EXPECT` header.
    #[inline]
    pub fn expect(&self) -> bool {
        self.flags.contains(Flags::EXPECT)
    }

    #[inline]
    pub(crate) fn set_expect(&mut self) {
        self.flags.insert(Flags::EXPECT);
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum RequestHeadType {
    Owned(RequestHead),
    Rc(Rc<RequestHead>, Option<HeaderMap>),
}

impl RequestHeadType {
    pub fn extra_headers(&self) -> Option<&HeaderMap> {
        match self {
            RequestHeadType::Owned(_) => None,
            RequestHeadType::Rc(_, headers) => headers.as_ref(),
        }
    }
}

impl AsRef<RequestHead> for RequestHeadType {
    fn as_ref(&self) -> &RequestHead {
        match self {
            RequestHeadType::Owned(head) => head,
            RequestHeadType::Rc(head, _) => head.as_ref(),
        }
    }
}

impl From<RequestHead> for RequestHeadType {
    fn from(head: RequestHead) -> Self {
        RequestHeadType::Owned(head)
    }
}

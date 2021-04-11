use std::cell::{Ref, RefCell, RefMut};
use std::net;
use std::rc::Rc;

use bitflags::bitflags;

use crate::extensions::Extensions;
use crate::header::HeaderMap;
use crate::http::{header, Method, StatusCode, Uri, Version};

/// Represents various types of connection
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ConnectionType {
    /// Close connection after response
    Close,

    /// Keep connection alive after response
    KeepAlive,

    /// Connection is upgraded to different type
    Upgrade,
}

bitflags! {
    pub(crate) struct Flags: u8 {
        const CLOSE       = 0b0000_0001;
        const KEEP_ALIVE  = 0b0000_0010;
        const UPGRADE     = 0b0000_0100;
        const EXPECT      = 0b0000_1000;
        const NO_CHUNKING = 0b0001_0000;
        const CAMEL_CASE  = 0b0010_0000;
    }
}

#[doc(hidden)]
pub trait Head: Default + 'static {
    fn clear(&mut self);

    fn with_pool<F, R>(f: F) -> R
    where
        F: FnOnce(&MessagePool<Self>) -> R;
}

#[derive(Debug)]
pub struct RequestHead {
    pub uri: Uri,
    pub method: Method,
    pub version: Version,
    pub headers: HeaderMap,
    pub extensions: RefCell<Extensions>,
    pub peer_addr: Option<net::SocketAddr>,
    flags: Flags,
}

impl Default for RequestHead {
    fn default() -> RequestHead {
        RequestHead {
            uri: Uri::default(),
            method: Method::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            flags: Flags::empty(),
            peer_addr: None,
            extensions: RefCell::new(Extensions::new()),
        }
    }
}

impl Head for RequestHead {
    fn clear(&mut self) {
        self.flags = Flags::empty();
        self.headers.clear();
        self.extensions.get_mut().clear();
    }

    fn with_pool<F, R>(f: F) -> R
    where
        F: FnOnce(&MessagePool<Self>) -> R,
    {
        REQUEST_POOL.with(|p| f(p))
    }
}

impl RequestHead {
    /// Message extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.extensions.borrow()
    }

    /// Mutable reference to a the message's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions.borrow_mut()
    }

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
        if let Some(hdr) = self.headers().get(header::CONNECTION) {
            if let Ok(s) = hdr.to_str() {
                s.to_ascii_lowercase().contains("upgrade")
            } else {
                false
            }
        } else {
            false
        }
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

    #[inline]
    /// Request contains `EXPECT` header
    pub fn expect(&self) -> bool {
        self.flags.contains(Flags::EXPECT)
    }

    #[inline]
    pub(crate) fn set_expect(&mut self) {
        self.flags.insert(Flags::EXPECT);
    }
}

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
            RequestHeadType::Owned(head) => &head,
            RequestHeadType::Rc(head, _) => head.as_ref(),
        }
    }
}

impl From<RequestHead> for RequestHeadType {
    fn from(head: RequestHead) -> Self {
        RequestHeadType::Owned(head)
    }
}

#[derive(Debug)]
pub struct ResponseHead {
    pub version: Version,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub reason: Option<&'static str>,
    pub(crate) extensions: RefCell<Extensions>,
    flags: Flags,
}

impl ResponseHead {
    /// Create new instance of `ResponseHead` type
    #[inline]
    pub fn new(status: StatusCode) -> ResponseHead {
        ResponseHead {
            status,
            version: Version::default(),
            headers: HeaderMap::with_capacity(12),
            reason: None,
            flags: Flags::empty(),
            extensions: RefCell::new(Extensions::new()),
        }
    }

    /// Message extensions
    #[inline]
    pub fn extensions(&self) -> Ref<'_, Extensions> {
        self.extensions.borrow()
    }

    /// Mutable reference to a the message's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<'_, Extensions> {
        self.extensions.borrow_mut()
    }

    #[inline]
    /// Read the message headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[inline]
    /// Mutable reference to the message headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
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

    #[inline]
    /// Check if keep-alive is enabled
    pub fn keep_alive(&self) -> bool {
        self.connection_type() == ConnectionType::KeepAlive
    }

    #[inline]
    /// Check upgrade status of this message
    pub fn upgrade(&self) -> bool {
        self.connection_type() == ConnectionType::Upgrade
    }

    /// Get custom reason for the response
    #[inline]
    pub fn reason(&self) -> &str {
        if let Some(reason) = self.reason {
            reason
        } else {
            self.status
                .canonical_reason()
                .unwrap_or("<unknown status code>")
        }
    }

    #[inline]
    pub(crate) fn ctype(&self) -> Option<ConnectionType> {
        if self.flags.contains(Flags::CLOSE) {
            Some(ConnectionType::Close)
        } else if self.flags.contains(Flags::KEEP_ALIVE) {
            Some(ConnectionType::KeepAlive)
        } else if self.flags.contains(Flags::UPGRADE) {
            Some(ConnectionType::Upgrade)
        } else {
            None
        }
    }

    #[inline]
    /// Get response body chunking state
    pub fn chunked(&self) -> bool {
        !self.flags.contains(Flags::NO_CHUNKING)
    }

    #[inline]
    /// Set no chunking for payload
    pub fn no_chunking(&mut self, val: bool) {
        if val {
            self.flags.insert(Flags::NO_CHUNKING);
        } else {
            self.flags.remove(Flags::NO_CHUNKING);
        }
    }
}

pub struct Message<T: Head> {
    /// Rc here should not be cloned by anyone.
    /// It's used to reuse allocation of T and no shared ownership is allowed.
    head: Rc<T>,
}

impl<T: Head> Message<T> {
    /// Get new message from the pool of objects
    pub fn new() -> Self {
        T::with_pool(|p| p.get_message())
    }
}

impl<T: Head> std::ops::Deref for Message<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.head.as_ref()
    }
}

impl<T: Head> std::ops::DerefMut for Message<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Rc::get_mut(&mut self.head).expect("Multiple copies exist")
    }
}

impl<T: Head> Drop for Message<T> {
    fn drop(&mut self) {
        T::with_pool(|p| p.release(self.head.clone()))
    }
}

pub(crate) struct BoxedResponseHead {
    head: Option<Box<ResponseHead>>,
}

impl BoxedResponseHead {
    /// Get new message from the pool of objects
    pub fn new(status: StatusCode) -> Self {
        RESPONSE_POOL.with(|p| p.get_message(status))
    }
}

impl std::ops::Deref for BoxedResponseHead {
    type Target = ResponseHead;

    fn deref(&self) -> &Self::Target {
        self.head.as_ref().unwrap()
    }
}

impl std::ops::DerefMut for BoxedResponseHead {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.head.as_mut().unwrap()
    }
}

impl Drop for BoxedResponseHead {
    fn drop(&mut self) {
        if let Some(head) = self.head.take() {
            RESPONSE_POOL.with(move |p| p.release(head))
        }
    }
}

#[doc(hidden)]
/// Request's objects pool
pub struct MessagePool<T: Head>(RefCell<Vec<Rc<T>>>);

#[doc(hidden)]
#[allow(clippy::vec_box)]
/// Request's objects pool
pub struct BoxedResponsePool(RefCell<Vec<Box<ResponseHead>>>);

thread_local!(static REQUEST_POOL: MessagePool<RequestHead> = MessagePool::<RequestHead>::create());
thread_local!(static RESPONSE_POOL: BoxedResponsePool = BoxedResponsePool::create());

impl<T: Head> MessagePool<T> {
    fn create() -> MessagePool<T> {
        MessagePool(RefCell::new(Vec::with_capacity(128)))
    }

    /// Get message from the pool
    #[inline]
    fn get_message(&self) -> Message<T> {
        if let Some(mut msg) = self.0.borrow_mut().pop() {
            // Message is put in pool only when it's the last copy.
            // which means it's guaranteed to be unique when popped out.
            Rc::get_mut(&mut msg)
                .expect("Multiple copies exist")
                .clear();
            Message { head: msg }
        } else {
            Message {
                head: Rc::new(T::default()),
            }
        }
    }

    #[inline]
    /// Release request instance
    fn release(&self, msg: Rc<T>) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            v.push(msg);
        }
    }
}

impl BoxedResponsePool {
    fn create() -> BoxedResponsePool {
        BoxedResponsePool(RefCell::new(Vec::with_capacity(128)))
    }

    /// Get message from the pool
    #[inline]
    fn get_message(&self, status: StatusCode) -> BoxedResponseHead {
        if let Some(mut head) = self.0.borrow_mut().pop() {
            head.reason = None;
            head.status = status;
            head.headers.clear();
            head.flags = Flags::empty();
            BoxedResponseHead { head: Some(head) }
        } else {
            BoxedResponseHead {
                head: Some(Box::new(ResponseHead::new(status))),
            }
        }
    }

    #[inline]
    /// Release request instance
    fn release(&self, mut msg: Box<ResponseHead>) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            msg.extensions.get_mut().clear();
            v.push(msg);
        }
    }
}

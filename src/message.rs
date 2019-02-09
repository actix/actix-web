use std::cell::{Ref, RefCell, RefMut};
use std::collections::VecDeque;
use std::rc::Rc;

use http::{HeaderMap, Method, StatusCode, Uri, Version};

use crate::extensions::Extensions;

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

#[doc(hidden)]
pub trait Head: Default + 'static {
    fn clear(&mut self);

    /// Connection type
    fn connection_type(&self) -> ConnectionType;

    /// Set connection type of the message
    fn set_connection_type(&mut self, ctype: ConnectionType);

    fn upgrade(&self) -> bool {
        self.connection_type() == ConnectionType::Upgrade
    }

    fn keep_alive(&self) -> bool {
        self.connection_type() == ConnectionType::KeepAlive
    }

    fn pool() -> &'static MessagePool<Self>;
}

#[derive(Debug)]
pub struct RequestHead {
    pub uri: Uri,
    pub method: Method,
    pub version: Version,
    pub headers: HeaderMap,
    pub ctype: Option<ConnectionType>,
    pub extensions: RefCell<Extensions>,
}

impl Default for RequestHead {
    fn default() -> RequestHead {
        RequestHead {
            uri: Uri::default(),
            method: Method::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            ctype: None,
            extensions: RefCell::new(Extensions::new()),
        }
    }
}

impl Head for RequestHead {
    fn clear(&mut self) {
        self.ctype = None;
        self.headers.clear();
        self.extensions.borrow_mut().clear();
    }

    fn set_connection_type(&mut self, ctype: ConnectionType) {
        self.ctype = Some(ctype)
    }

    fn connection_type(&self) -> ConnectionType {
        if let Some(ct) = self.ctype {
            ct
        } else if self.version < Version::HTTP_11 {
            ConnectionType::Close
        } else {
            ConnectionType::KeepAlive
        }
    }

    fn pool() -> &'static MessagePool<Self> {
        REQUEST_POOL.with(|p| *p)
    }
}

impl RequestHead {
    /// Message extensions
    #[inline]
    pub fn extensions(&self) -> Ref<Extensions> {
        self.extensions.borrow()
    }

    /// Mutable reference to a the message's extensions
    #[inline]
    pub fn extensions_mut(&self) -> RefMut<Extensions> {
        self.extensions.borrow_mut()
    }
}

#[derive(Debug)]
pub struct ResponseHead {
    pub version: Version,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub reason: Option<&'static str>,
    pub(crate) ctype: Option<ConnectionType>,
}

impl Default for ResponseHead {
    fn default() -> ResponseHead {
        ResponseHead {
            version: Version::default(),
            status: StatusCode::OK,
            headers: HeaderMap::with_capacity(16),
            reason: None,
            ctype: None,
        }
    }
}

impl Head for ResponseHead {
    fn clear(&mut self) {
        self.ctype = None;
        self.reason = None;
        self.headers.clear();
    }

    fn set_connection_type(&mut self, ctype: ConnectionType) {
        self.ctype = Some(ctype)
    }

    fn connection_type(&self) -> ConnectionType {
        if let Some(ct) = self.ctype {
            ct
        } else if self.version < Version::HTTP_11 {
            ConnectionType::Close
        } else {
            ConnectionType::KeepAlive
        }
    }

    fn pool() -> &'static MessagePool<Self> {
        RESPONSE_POOL.with(|p| *p)
    }
}

impl ResponseHead {
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
}

pub struct Message<T: Head> {
    head: Rc<T>,
    pool: &'static MessagePool<T>,
}

impl<T: Head> Message<T> {
    /// Get new message from the pool of objects
    pub fn new() -> Self {
        T::pool().get_message()
    }
}

impl<T: Head> Clone for Message<T> {
    fn clone(&self) -> Self {
        Message {
            head: self.head.clone(),
            pool: self.pool,
        }
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
        if Rc::strong_count(&self.head) == 1 {
            self.pool.release(self.head.clone());
        }
    }
}

#[doc(hidden)]
/// Request's objects pool
pub struct MessagePool<T: Head>(RefCell<VecDeque<Rc<T>>>);

thread_local!(static REQUEST_POOL: &'static MessagePool<RequestHead> = MessagePool::<RequestHead>::create());
thread_local!(static RESPONSE_POOL: &'static MessagePool<ResponseHead> = MessagePool::<ResponseHead>::create());

impl<T: Head> MessagePool<T> {
    fn create() -> &'static MessagePool<T> {
        let pool = MessagePool(RefCell::new(VecDeque::with_capacity(128)));
        Box::leak(Box::new(pool))
    }

    /// Get message from the pool
    #[inline]
    fn get_message(&'static self) -> Message<T> {
        if let Some(mut msg) = self.0.borrow_mut().pop_front() {
            if let Some(r) = Rc::get_mut(&mut msg) {
                r.clear();
            }
            Message {
                head: msg,
                pool: self,
            }
        } else {
            Message {
                head: Rc::new(T::default()),
                pool: self,
            }
        }
    }

    #[inline]
    /// Release request instance
    fn release(&self, msg: Rc<T>) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            v.push_front(msg);
        }
    }
}

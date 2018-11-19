use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use http::{HeaderMap, Method, StatusCode, Uri, Version};

use extensions::Extensions;
use payload::Payload;
use uri::Url;

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

pub struct RequestHead {
    pub uri: Uri,
    pub method: Method,
    pub version: Version,
    pub headers: HeaderMap,
    ctype: Option<ConnectionType>,
}

impl Default for RequestHead {
    fn default() -> RequestHead {
        RequestHead {
            uri: Uri::default(),
            method: Method::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            ctype: None,
        }
    }
}

impl Head for RequestHead {
    fn clear(&mut self) {
        self.ctype = None;
        self.headers.clear();
    }

    fn set_connection_type(&mut self, ctype: ConnectionType) {
        self.ctype = Some(ctype)
    }

    fn connection_type(&self) -> ConnectionType {
        if let Some(ct) = self.ctype {
            ct
        } else if self.version <= Version::HTTP_11 {
            ConnectionType::Close
        } else {
            ConnectionType::KeepAlive
        }
    }

    fn pool() -> &'static MessagePool<Self> {
        REQUEST_POOL.with(|p| *p)
    }
}

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
        } else if self.version <= Version::HTTP_11 {
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
    pub head: T,
    pub url: Url,
    pub status: StatusCode,
    pub extensions: RefCell<Extensions>,
    pub payload: RefCell<Option<Payload>>,
    pub(crate) pool: &'static MessagePool<T>,
}

impl<T: Head> Message<T> {
    #[inline]
    /// Reset request instance
    pub fn reset(&mut self) {
        self.head.clear();
        self.extensions.borrow_mut().clear();
        *self.payload.borrow_mut() = None;
    }
}

impl<T: Head> Default for Message<T> {
    fn default() -> Self {
        Message {
            pool: T::pool(),
            url: Url::default(),
            head: T::default(),
            status: StatusCode::OK,
            payload: RefCell::new(None),
            extensions: RefCell::new(Extensions::new()),
        }
    }
}

#[doc(hidden)]
/// Request's objects pool
pub struct MessagePool<T: Head>(RefCell<VecDeque<Rc<Message<T>>>>);

thread_local!(static REQUEST_POOL: &'static MessagePool<RequestHead> = MessagePool::<RequestHead>::create());
thread_local!(static RESPONSE_POOL: &'static MessagePool<ResponseHead> = MessagePool::<ResponseHead>::create());

impl MessagePool<RequestHead> {
    /// Get default request's pool
    pub fn pool() -> &'static MessagePool<RequestHead> {
        REQUEST_POOL.with(|p| *p)
    }

    /// Get Request object
    #[inline]
    pub fn get_message() -> Rc<Message<RequestHead>> {
        REQUEST_POOL.with(|pool| {
            if let Some(mut msg) = pool.0.borrow_mut().pop_front() {
                if let Some(r) = Rc::get_mut(&mut msg) {
                    r.reset();
                }
                return msg;
            }
            Rc::new(Message::default())
        })
    }
}

impl<T: Head> MessagePool<T> {
    fn create() -> &'static MessagePool<T> {
        let pool = MessagePool(RefCell::new(VecDeque::with_capacity(128)));
        Box::leak(Box::new(pool))
    }

    #[inline]
    /// Release request instance
    pub(crate) fn release(&self, msg: Rc<Message<T>>) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            v.push_front(msg);
        }
    }
}

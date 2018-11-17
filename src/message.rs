use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use http::{HeaderMap, Method, StatusCode, Uri, Version};

use extensions::Extensions;
use payload::Payload;
use uri::Url;

#[doc(hidden)]
pub trait Head: Default + 'static {
    fn clear(&mut self);

    fn pool() -> &'static MessagePool<Self>;
}

bitflags! {
    pub(crate) struct MessageFlags: u8 {
        const KEEPALIVE = 0b0000_0001;
    }
}

pub struct RequestHead {
    pub uri: Uri,
    pub method: Method,
    pub version: Version,
    pub headers: HeaderMap,
    pub(crate) flags: MessageFlags,
}

impl Default for RequestHead {
    fn default() -> RequestHead {
        RequestHead {
            uri: Uri::default(),
            method: Method::default(),
            version: Version::HTTP_11,
            headers: HeaderMap::with_capacity(16),
            flags: MessageFlags::empty(),
        }
    }
}

impl Head for RequestHead {
    fn clear(&mut self) {
        self.headers.clear();
        self.flags = MessageFlags::empty();
    }

    fn pool() -> &'static MessagePool<Self> {
        REQUEST_POOL.with(|p| *p)
    }
}

pub struct ResponseHead {
    pub version: Option<Version>,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub reason: Option<&'static str>,
    pub(crate) flags: MessageFlags,
}

impl Default for ResponseHead {
    fn default() -> ResponseHead {
        ResponseHead {
            version: None,
            status: StatusCode::OK,
            headers: HeaderMap::with_capacity(16),
            reason: None,
            flags: MessageFlags::empty(),
        }
    }
}

impl Head for ResponseHead {
    fn clear(&mut self) {
        self.reason = None;
        self.version = None;
        self.headers.clear();
        self.flags = MessageFlags::empty();
    }

    fn pool() -> &'static MessagePool<Self> {
        RESPONSE_POOL.with(|p| *p)
    }
}

pub struct Message<T: Head> {
    pub head: T,
    pub url: Url,
    pub status: StatusCode,
    pub extensions: RefCell<Extensions>,
    pub payload: RefCell<Option<Payload>>,
    pub(crate) pool: &'static MessagePool<T>,
    pub(crate) flags: Cell<MessageFlags>,
}

impl<T: Head> Message<T> {
    #[inline]
    /// Reset request instance
    pub fn reset(&mut self) {
        self.head.clear();
        self.extensions.borrow_mut().clear();
        self.flags.set(MessageFlags::empty());
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
            flags: Cell::new(MessageFlags::empty()),
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

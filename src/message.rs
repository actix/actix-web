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

    fn flags(&self) -> MessageFlags;

    fn flags_mut(&mut self) -> &mut MessageFlags;

    fn pool() -> &'static MessagePool<Self>;

    /// Set upgrade
    fn set_upgrade(&mut self) {
        *self.flags_mut() = MessageFlags::UPGRADE;
    }

    /// Check if request is upgrade request
    fn upgrade(&self) -> bool {
        self.flags().contains(MessageFlags::UPGRADE)
    }

    /// Set keep-alive
    fn set_keep_alive(&mut self) {
        *self.flags_mut() = MessageFlags::KEEP_ALIVE;
    }

    /// Check if request is keep-alive
    fn keep_alive(&self) -> bool;

    /// Set force-close connection
    fn force_close(&mut self) {
        *self.flags_mut() = MessageFlags::FORCE_CLOSE;
    }
}

bitflags! {
    pub struct MessageFlags: u8 {
        const KEEP_ALIVE  = 0b0000_0001;
        const FORCE_CLOSE = 0b0000_0010;
        const UPGRADE     = 0b0000_0100;
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

    fn flags(&self) -> MessageFlags {
        self.flags
    }

    fn flags_mut(&mut self) -> &mut MessageFlags {
        &mut self.flags
    }

    /// Check if request is keep-alive
    fn keep_alive(&self) -> bool {
        if self.flags().contains(MessageFlags::FORCE_CLOSE) {
            false
        } else if self.flags().contains(MessageFlags::KEEP_ALIVE) {
            true
        } else {
            self.version <= Version::HTTP_11
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
    pub(crate) flags: MessageFlags,
}

impl Default for ResponseHead {
    fn default() -> ResponseHead {
        ResponseHead {
            version: Version::default(),
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
        self.headers.clear();
        self.flags = MessageFlags::empty();
    }

    fn flags(&self) -> MessageFlags {
        self.flags
    }

    fn flags_mut(&mut self) -> &mut MessageFlags {
        &mut self.flags
    }

    /// Check if response is keep-alive
    fn keep_alive(&self) -> bool {
        if self.flags().contains(MessageFlags::FORCE_CLOSE) {
            false
        } else if self.flags().contains(MessageFlags::KEEP_ALIVE) {
            true
        } else {
            self.version <= Version::HTTP_11
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

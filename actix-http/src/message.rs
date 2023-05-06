use std::{cell::RefCell, ops, rc::Rc};

use bitflags::bitflags;

/// Represents various types of connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionType {
    /// Close connection after response.
    Close,

    /// Keep connection alive after response.
    KeepAlive,

    /// Connection is upgraded to different type.
    Upgrade,
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
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

pub struct Message<T: Head> {
    /// Rc here should not be cloned by anyone.
    /// It's used to reuse allocation of T and no shared ownership is allowed.
    head: Rc<T>,
}

impl<T: Head> Message<T> {
    /// Get new message from the pool of objects
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        T::with_pool(MessagePool::get_message)
    }
}

impl<T: Head> ops::Deref for Message<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.head.as_ref()
    }
}

impl<T: Head> ops::DerefMut for Message<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Rc::get_mut(&mut self.head).expect("Multiple copies exist")
    }
}

impl<T: Head> Drop for Message<T> {
    fn drop(&mut self) {
        T::with_pool(|p| p.release(self.head.clone()))
    }
}

/// Generic `Head` object pool.
#[doc(hidden)]
pub struct MessagePool<T: Head>(RefCell<Vec<Rc<T>>>);

impl<T: Head> MessagePool<T> {
    pub(crate) fn create() -> MessagePool<T> {
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
    /// Release message instance
    fn release(&self, msg: Rc<T>) {
        let pool = &mut self.0.borrow_mut();
        if pool.len() < 128 {
            pool.push(msg);
        }
    }
}

use bytes::{BufMut, BytesMut};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;

use body::Binary;

/// Internal use only! unsafe
#[derive(Debug)]
pub(crate) struct SharedBytesPool(RefCell<VecDeque<Rc<BytesMut>>>);

impl SharedBytesPool {
    pub fn new() -> SharedBytesPool {
        SharedBytesPool(RefCell::new(VecDeque::with_capacity(128)))
    }

    pub fn get_bytes(&self) -> Rc<BytesMut> {
        if let Some(bytes) = self.0.borrow_mut().pop_front() {
            bytes
        } else {
            Rc::new(BytesMut::new())
        }
    }

    pub fn release_bytes(&self, mut bytes: Rc<BytesMut>) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            Rc::get_mut(&mut bytes).unwrap().clear();
            v.push_front(bytes);
        }
    }
}

#[derive(Debug)]
pub(crate) struct SharedBytes(Option<Rc<BytesMut>>, Option<Rc<SharedBytesPool>>);

impl Drop for SharedBytes {
    fn drop(&mut self) {
        if let Some(pool) = self.1.take() {
            if let Some(bytes) = self.0.take() {
                if Rc::strong_count(&bytes) == 1 {
                    pool.release_bytes(bytes);
                }
            }
        }
    }
}

impl SharedBytes {
    pub fn new(bytes: Rc<BytesMut>, pool: Rc<SharedBytesPool>) -> SharedBytes {
        SharedBytes(Some(bytes), Some(pool))
    }

    #[inline(always)]
    #[allow(mutable_transmutes)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref, inline_always))]
    pub(crate) fn get_mut(&self) -> &mut BytesMut {
        let r: &BytesMut = self.0.as_ref().unwrap().as_ref();
        unsafe { &mut *(r as *const _ as *mut _) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.as_ref().unwrap().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.as_ref().unwrap().is_empty()
    }

    #[inline]
    pub fn as_ref(&self) -> &[u8] {
        self.0.as_ref().unwrap().as_ref()
    }

    pub fn split_to(&self, n: usize) -> BytesMut {
        self.get_mut().split_to(n)
    }

    pub fn take(&self) -> BytesMut {
        self.get_mut().take()
    }

    #[inline]
    #[cfg_attr(feature = "cargo-clippy", allow(needless_pass_by_value))]
    pub fn extend(&self, data: Binary) {
        let buf = self.get_mut();
        let data = data.as_ref();
        buf.reserve(data.len());
        SharedBytes::put_slice(buf, data);
    }

    #[inline]
    pub fn extend_from_slice(&self, data: &[u8]) {
        let buf = self.get_mut();
        buf.reserve(data.len());
        SharedBytes::put_slice(buf, data);
    }

    #[inline]
    pub(crate) fn put_slice(buf: &mut BytesMut, src: &[u8]) {
        let len = src.len();
        unsafe {
            buf.bytes_mut()[..len].copy_from_slice(src);
            buf.advance_mut(len);
        }
    }

    #[inline]
    pub(crate) fn extend_from_slice_(buf: &mut BytesMut, data: &[u8]) {
        buf.reserve(data.len());
        SharedBytes::put_slice(buf, data);
    }
}

impl Default for SharedBytes {
    fn default() -> Self {
        SharedBytes(Some(Rc::new(BytesMut::new())), None)
    }
}

impl Clone for SharedBytes {
    fn clone(&self) -> SharedBytes {
        SharedBytes(self.0.clone(), self.1.clone())
    }
}

impl io::Write for SharedBytes {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

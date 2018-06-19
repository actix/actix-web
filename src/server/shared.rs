use bytes::{BufMut, BytesMut};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;

use body::Binary;

#[derive(Debug)]
pub(crate) struct SharedBytesPool(RefCell<VecDeque<BytesMut>>);

impl SharedBytesPool {
    pub fn new() -> SharedBytesPool {
        SharedBytesPool(RefCell::new(VecDeque::with_capacity(128)))
    }

    pub fn get_bytes(&self) -> BytesMut {
        if let Some(bytes) = self.0.borrow_mut().pop_front() {
            bytes
        } else {
            BytesMut::new()
        }
    }

    pub fn release_bytes(&self, mut bytes: BytesMut) {
        let v = &mut self.0.borrow_mut();
        if v.len() < 128 {
            bytes.clear();
            v.push_front(bytes);
        }
    }
}

#[derive(Debug)]
pub(crate) struct SharedBytes(Option<BytesMut>, Option<Rc<SharedBytesPool>>);

impl Drop for SharedBytes {
    fn drop(&mut self) {
        if let Some(pool) = self.1.take() {
            if let Some(bytes) = self.0.take() {
                pool.release_bytes(bytes);
            }
        }
    }
}

impl SharedBytes {
    pub fn new(bytes: BytesMut, pool: Rc<SharedBytesPool>) -> SharedBytes {
        SharedBytes(Some(bytes), Some(pool))
    }

    #[inline]
    pub(crate) fn get_mut(&mut self) -> &mut BytesMut {
        self.0.as_mut().unwrap()
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

    pub fn split_to(&mut self, n: usize) -> BytesMut {
        self.get_mut().split_to(n)
    }

    pub fn take(&mut self) -> BytesMut {
        self.get_mut().take()
    }

    #[inline]
    pub fn extend(&mut self, data: &Binary) {
        let buf = self.get_mut();
        let data = data.as_ref();
        buf.reserve(data.len());
        SharedBytes::put_slice(buf, data);
    }

    #[inline]
    pub fn extend_from_slice(&mut self, data: &[u8]) {
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
        SharedBytes(Some(BytesMut::new()), None)
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

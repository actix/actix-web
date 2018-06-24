use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;

use bytes::BytesMut;

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

    pub fn empty() -> SharedBytes {
        SharedBytes(Some(BytesMut::new()), None)
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
    pub fn reserve(&mut self, cap: usize) {
        self.get_mut().reserve(cap);
    }

    #[inline]
    pub fn extend_from_slice(&mut self, data: &[u8]) {
        let buf = self.get_mut();
        buf.extend_from_slice(data);
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

use std::{cmp, mem};
use std::cell::RefCell;
use std::rc::Rc;
use std::collections::VecDeque;
use iovec::IoVec;
use bytes::{Buf, Bytes, BytesMut};

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
            Rc::get_mut(&mut bytes).unwrap().take();
            v.push_front(bytes);
        }
    }
}

#[derive(Debug)]
pub(crate) struct SharedBytes(
    Option<Rc<BytesMut>>, Option<Rc<SharedBytesPool>>);

impl Drop for SharedBytes {
    fn drop(&mut self) {
        if let Some(ref pool) = self.1 {
            if let Some(bytes) = self.0.take() {
                if Rc::strong_count(&bytes) == 1 {
                    pool.release_bytes(bytes);
                }
            }
        }
    }
}

impl SharedBytes {

    pub fn empty() -> Self {
        SharedBytes(None, None)
    }

    pub fn new(bytes: Rc<BytesMut>, pool: Rc<SharedBytesPool>) -> SharedBytes {
        SharedBytes(Some(bytes), Some(pool))
    }

    #[inline(always)]
    #[allow(mutable_transmutes)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref, inline_always))]
    pub fn get_mut(&self) -> &mut BytesMut {
        let r: &BytesMut = self.0.as_ref().unwrap().as_ref();
        unsafe{mem::transmute(r)}
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
        self.get_mut().extend_from_slice(data.as_ref());
    }

    #[inline]
    pub fn extend_from_slice(&self, data: &[u8]) {
        self.get_mut().extend_from_slice(data);
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


#[derive(Debug)]
pub(crate) struct SharedIo(
    Rc<VecDeque<Binary>>
);

impl SharedIo {
    #[inline(always)]
    #[allow(mutable_transmutes)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref, inline_always))]
    fn get_mut(&self) -> &mut VecDeque<Binary> {
        let r: &VecDeque<_> = self.0.as_ref();
        unsafe{mem::transmute(r)}
    }

    pub fn clear(&self) {
        self.get_mut().clear();
    }

    pub fn push(&self, data: Binary) {
        self.get_mut().push_back(data);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn split_to(&self, n: usize) -> Bytes {
        let b = Bytes::from(&self.bytes()[..n]);

        let slf: &mut SharedIo = unsafe{mem::transmute(self as *const _ as *mut SharedIo)};
        slf.advance(n);
        b
    }

    pub fn take(&self) -> Bytes {
        match self.0.len() {
            0 => Bytes::from_static(b""),
            1 => self.get_mut().pop_front().unwrap().into(),
            _ => {
                self.squash();
                self.take()
            }
        }
    }

    fn squash(&self) {
        let len = self.remaining();
        let buf = self.0.iter().fold(
            BytesMut::with_capacity(len),
            |mut buf, item| {buf.extend_from_slice(item.as_ref()); buf});
        let vec = self.get_mut();
        vec.clear();
        vec.push_back(buf.into());
    }
}

impl Default for SharedIo {
    fn default() -> SharedIo {
        SharedIo(Rc::new(VecDeque::new()))
    }
}

impl Clone for SharedIo {
    fn clone(&self) -> SharedIo {
        SharedIo(Rc::clone(&self.0))
    }
}

impl Buf for SharedIo {
    fn remaining(&self) -> usize {
        self.0.iter().fold(0, |cnt, item| cnt + item.len())
    }

    fn bytes(&self) -> &[u8] {
        match self.0.len() {
            0 => b"",
            1 => self.0[0].as_ref(),
            _ => {
                self.squash();
                self.bytes()
            }
        }
    }

    fn bytes_vec<'a>(&'a self, dst: &mut [&'a IoVec]) -> usize {
        let num = cmp::min(dst.len(), self.0.len());
        for idx in 0..num {
            dst[idx] = self.0[idx].as_ref().into();
        }
        num
    }

    fn advance(&mut self, mut cnt: usize) {
        let vec = self.get_mut();
        while cnt > 0 {
            if let Some(mut item) = vec.pop_front() {
                if item.len() <= cnt {
                    cnt -= item.len();
                } else {
                    let mut item = item.take();
                    item.split_to(cnt);
                    vec.push_front(item.into());
                    break
                }
            } else {
                break
            }
        }
    }
}

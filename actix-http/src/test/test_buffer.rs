use std::{
    cell::{Ref, RefCell, RefMut},
    io::{self, Read, Write},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use bytes::{Bytes, BytesMut};

/// Async I/O test buffer.
#[derive(Debug)]
pub struct TestBuffer {
    pub read_buf: Rc<RefCell<BytesMut>>,
    pub write_buf: Rc<RefCell<BytesMut>>,
    pub err: Option<Rc<io::Error>>,
}

impl TestBuffer {
    /// Create new `TestBuffer` instance with initial read buffer.
    pub fn new<T>(data: T) -> Self
    where
        T: Into<BytesMut>,
    {
        Self {
            read_buf: Rc::new(RefCell::new(data.into())),
            write_buf: Rc::new(RefCell::new(BytesMut::new())),
            err: None,
        }
    }

    // intentionally not using Clone trait
    #[allow(dead_code)]
    pub(crate) fn clone(&self) -> Self {
        Self {
            read_buf: Rc::clone(&self.read_buf),
            write_buf: Rc::clone(&self.write_buf),
            err: self.err.clone(),
        }
    }

    /// Create new empty `TestBuffer` instance.
    pub fn empty() -> Self {
        Self::new("")
    }

    #[allow(dead_code)]
    pub(crate) fn read_buf_slice(&self) -> Ref<'_, [u8]> {
        Ref::map(self.read_buf.borrow(), |b| b.as_ref())
    }

    #[allow(dead_code)]
    pub(crate) fn read_buf_slice_mut(&self) -> RefMut<'_, [u8]> {
        RefMut::map(self.read_buf.borrow_mut(), |b| b.as_mut())
    }

    #[allow(dead_code)]
    pub(crate) fn write_buf_slice(&self) -> Ref<'_, [u8]> {
        Ref::map(self.write_buf.borrow(), |b| b.as_ref())
    }

    #[allow(dead_code)]
    pub(crate) fn write_buf_slice_mut(&self) -> RefMut<'_, [u8]> {
        RefMut::map(self.write_buf.borrow_mut(), |b| b.as_mut())
    }

    #[allow(dead_code)]
    pub(crate) fn take_write_buf(&self) -> Bytes {
        self.write_buf.borrow_mut().split().freeze()
    }

    /// Add data to read buffer.
    pub fn extend_read_buf<T: AsRef<[u8]>>(&mut self, data: T) {
        self.read_buf.borrow_mut().extend_from_slice(data.as_ref())
    }
}

impl io::Read for TestBuffer {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
        if self.read_buf.borrow().is_empty() {
            if self.err.is_some() {
                Err(Rc::try_unwrap(self.err.take().unwrap()).unwrap())
            } else {
                Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
            }
        } else {
            let size = std::cmp::min(self.read_buf.borrow().len(), dst.len());
            let b = self.read_buf.borrow_mut().split_to(size);
            dst[..size].copy_from_slice(&b);
            Ok(size)
        }
    }
}

impl io::Write for TestBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_buf.borrow_mut().extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncRead for TestBuffer {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let dst = buf.initialize_unfilled();
        let res = self.get_mut().read(dst).map(|n| buf.advance(n));
        Poll::Ready(res)
    }
}

impl AsyncWrite for TestBuffer {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(self.get_mut().write(buf))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

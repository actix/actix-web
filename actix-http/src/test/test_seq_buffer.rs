use std::{
    cell::{Ref, RefCell},
    io::{self, Read, Write},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use bytes::{Bytes, BytesMut};

/// Async I/O test buffer with ability to incrementally add to the read buffer.
#[derive(Clone)]
pub struct TestSeqBuffer(Rc<RefCell<TestSeqInner>>);

impl TestSeqBuffer {
    /// Create new `TestBuffer` instance with initial read buffer.
    pub fn new<T>(data: T) -> Self
    where
        T: Into<BytesMut>,
    {
        Self(Rc::new(RefCell::new(TestSeqInner {
            read_buf: data.into(),
            read_closed: false,
            write_buf: BytesMut::new(),
            err: None,
        })))
    }

    /// Create new empty `TestBuffer` instance.
    pub fn empty() -> Self {
        Self::new(BytesMut::new())
    }

    pub fn read_buf(&self) -> Ref<'_, BytesMut> {
        Ref::map(self.0.borrow(), |inner| &inner.read_buf)
    }

    pub fn write_buf(&self) -> Ref<'_, BytesMut> {
        Ref::map(self.0.borrow(), |inner| &inner.write_buf)
    }

    pub fn take_write_buf(&self) -> Bytes {
        self.0.borrow_mut().write_buf.split().freeze()
    }

    pub fn err(&self) -> Ref<'_, Option<io::Error>> {
        Ref::map(self.0.borrow(), |inner| &inner.err)
    }

    /// Add data to read buffer.
    ///
    /// # Panics
    ///
    /// Panics if called after [`TestSeqBuffer::close_read`] has been called
    pub fn extend_read_buf<T: AsRef<[u8]>>(&mut self, data: T) {
        let mut inner = self.0.borrow_mut();
        if inner.read_closed {
            panic!("Tried to extend the read buffer after calling close_read");
        }

        inner.read_buf.extend_from_slice(data.as_ref())
    }

    /// Closes the [`AsyncRead`]/[`Read`] part of this test buffer.
    ///
    /// The current data in the buffer will still be returned by a call to read/poll_read, however,
    /// after the buffer is empty, it will return `Ok(0)` to signify the EOF condition
    pub fn close_read(&self) {
        self.0.borrow_mut().read_closed = true;
    }
}

pub struct TestSeqInner {
    read_buf: BytesMut,
    read_closed: bool,
    write_buf: BytesMut,
    err: Option<io::Error>,
}

impl io::Read for TestSeqBuffer {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
        let mut inner = self.0.borrow_mut();

        if inner.read_buf.is_empty() {
            if let Some(err) = inner.err.take() {
                Err(err)
            } else if inner.read_closed {
                Ok(0)
            } else {
                Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
            }
        } else {
            let size = std::cmp::min(inner.read_buf.len(), dst.len());
            let b = inner.read_buf.split_to(size);
            dst[..size].copy_from_slice(&b);
            Ok(size)
        }
    }
}

impl io::Write for TestSeqBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write_buf.extend(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncRead for TestSeqBuffer {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let dst = buf.initialize_unfilled();
        let r = self.get_mut().read(dst);
        match r {
            Ok(n) => {
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

impl AsyncWrite for TestSeqBuffer {
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

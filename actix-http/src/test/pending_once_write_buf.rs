use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use bytes::BytesMut;

use super::TestBuffer;

pub(crate) struct PendingOnceWriteBuf {
    io: TestBuffer,
    block_next_write: bool,
}

impl PendingOnceWriteBuf {
    pub(crate) fn new<T>(data: T) -> Self
    where
        T: Into<BytesMut>,
    {
        Self {
            io: TestBuffer::new(data),
            block_next_write: true,
        }
    }
}

impl io::Read for PendingOnceWriteBuf {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
        self.io.read(dst)
    }
}

impl io::Write for PendingOnceWriteBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.io.flush()
    }
}

impl AsyncRead for PendingOnceWriteBuf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for PendingOnceWriteBuf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.block_next_write {
            self.block_next_write = false;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }

        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

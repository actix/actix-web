use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use actix_codec::{AsyncRead, AsyncWrite, ReadBuf};
use bytes::BytesMut;

use super::TestBuffer;

/// Test I/O buffer that returns `Poll::Pending` on its first write.
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

#[cfg(test)]
mod tests {
    use std::{io, task::Context};

    use bytes::Bytes;
    use futures_util::task::noop_waker_ref;
    use tokio_test::{assert_pending, assert_ready_eq, assert_ready_ok};

    use super::*;

    #[test]
    fn io_operations() {
        let mut buffer = PendingOnceWriteBuf::new("read");
        let mut read = [0; 4];
        assert_eq!(io::Read::read(&mut buffer, &mut read).unwrap(), 4);
        assert_eq!(&read, b"read");

        assert_eq!(io::Write::write(&mut buffer, b"write").unwrap(), 5);
        io::Write::flush(&mut buffer).unwrap();
        assert_eq!(buffer.io.take_write_buf(), Bytes::from_static(b"write"));
    }

    #[test]
    fn async_operations() {
        let mut cx = Context::from_waker(noop_waker_ref());
        let mut buffer = PendingOnceWriteBuf::new("read");

        let mut read = [0; 4];
        let mut read_buf = ReadBuf::new(&mut read);
        assert_ready_ok!(Pin::new(&mut buffer).poll_read(&mut cx, &mut read_buf));
        assert_eq!(read_buf.filled(), b"read");

        assert_pending!(Pin::new(&mut buffer).poll_write(&mut cx, b"write"));
        assert_ready_eq!(
            Pin::new(&mut buffer)
                .poll_write(&mut cx, b"write")
                .map(|result| result.unwrap()),
            5
        );
        assert_ready_ok!(Pin::new(&mut buffer).poll_flush(&mut cx));
        assert_ready_ok!(Pin::new(&mut buffer).poll_shutdown(&mut cx));
    }
}

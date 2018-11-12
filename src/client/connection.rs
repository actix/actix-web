use std::{fmt, io, time};

use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};

use super::pool::Acquired;

/// HTTP client connection
pub struct Connection<T: AsyncRead + AsyncWrite + 'static> {
    io: T,
    created: time::Instant,
    pool: Option<Acquired<T>>,
}

impl<T> fmt::Debug for Connection<T>
where
    T: AsyncRead + AsyncWrite + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Connection {:?}", self.io)
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> Connection<T> {
    pub(crate) fn new(io: T, created: time::Instant, pool: Acquired<T>) -> Self {
        Connection {
            io,
            created,
            pool: Some(pool),
        }
    }

    /// Raw IO stream
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.io
    }

    /// Close connection
    pub fn close(mut self) {
        if let Some(mut pool) = self.pool.take() {
            pool.close(self)
        }
    }

    /// Release this connection to the connection pool
    pub fn release(mut self) {
        if let Some(mut pool) = self.pool.take() {
            pool.release(self)
        }
    }

    pub(crate) fn into_inner(self) -> (T, time::Instant) {
        (self.io, self.created)
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> io::Read for Connection<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io.read(buf)
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> AsyncRead for Connection<T> {}

impl<T: AsyncRead + AsyncWrite + 'static> io::Write for Connection<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.io.flush()
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> AsyncWrite for Connection<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.io.shutdown()
    }
}

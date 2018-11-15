use std::{fmt, io, time};

use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};

use super::pool::Acquired;

pub trait Connection: AsyncRead + AsyncWrite + 'static {
    /// Close connection
    fn close(&mut self);

    /// Release connection to the connection pool
    fn release(&mut self);
}

#[doc(hidden)]
/// HTTP client connection
pub struct IoConnection<T> {
    io: Option<T>,
    created: time::Instant,
    pool: Option<Acquired<T>>,
}

impl<T> fmt::Debug for IoConnection<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Connection {:?}", self.io)
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> IoConnection<T> {
    pub(crate) fn new(io: T, created: time::Instant, pool: Acquired<T>) -> Self {
        IoConnection {
            created,
            io: Some(io),
            pool: Some(pool),
        }
    }

    /// Raw IO stream
    pub fn get_mut(&mut self) -> &mut T {
        self.io.as_mut().unwrap()
    }

    pub(crate) fn into_inner(self) -> (T, time::Instant) {
        (self.io.unwrap(), self.created)
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> Connection for IoConnection<T> {
    /// Close connection
    fn close(&mut self) {
        if let Some(mut pool) = self.pool.take() {
            if let Some(io) = self.io.take() {
                pool.close(IoConnection {
                    io: Some(io),
                    created: self.created,
                    pool: None,
                })
            }
        }
    }

    /// Release this connection to the connection pool
    fn release(&mut self) {
        if let Some(mut pool) = self.pool.take() {
            if let Some(io) = self.io.take() {
                pool.release(IoConnection {
                    io: Some(io),
                    created: self.created,
                    pool: None,
                })
            }
        }
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> io::Read for IoConnection<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io.as_mut().unwrap().read(buf)
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> AsyncRead for IoConnection<T> {}

impl<T: AsyncRead + AsyncWrite + 'static> io::Write for IoConnection<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io.as_mut().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.io.as_mut().unwrap().flush()
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> AsyncWrite for IoConnection<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.io.as_mut().unwrap().shutdown()
    }
}

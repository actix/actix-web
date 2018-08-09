use std::net::Shutdown;
use std::{io, time};

use futures::{Async, Future, Poll};
use native_tls::{self, TlsAcceptor, HandshakeError};
use tokio_io::{AsyncRead, AsyncWrite};

use server::{AcceptorService, IoStream};

#[derive(Clone)]
/// Support `SSL` connections via native-tls package
///
/// `tls` feature enables `NativeTlsAcceptor` type
pub struct NativeTlsAcceptor {
    acceptor: TlsAcceptor,
}

/// A wrapper around an underlying raw stream which implements the TLS or SSL
/// protocol.
///
/// A `TlsStream<S>` represents a handshake that has been completed successfully
/// and both the server and the client are ready for receiving and sending
/// data. Bytes read from a `TlsStream` are decrypted from `S` and bytes written
/// to a `TlsStream` are encrypted when passing through to `S`.
#[derive(Debug)]
pub struct TlsStream<S> {
    inner: native_tls::TlsStream<S>,
}

/// Future returned from `NativeTlsAcceptor::accept` which will resolve
/// once the accept handshake has finished.
pub struct Accept<S>{
    inner: Option<Result<native_tls::TlsStream<S>, HandshakeError<S>>>,
}

impl NativeTlsAcceptor {
    /// Create `NativeTlsAcceptor` instance
    pub fn new(acceptor: TlsAcceptor) -> Self {
        NativeTlsAcceptor { acceptor: acceptor.into() }
    }
}

impl<Io: IoStream> AcceptorService<Io> for NativeTlsAcceptor {
    type Accepted = TlsStream<Io>;
    type Future = Accept<Io>;

    fn scheme(&self) -> &'static str {
        "https"
    }

    fn accept(&self, io: Io) -> Self::Future {
        Accept { inner: Some(self.acceptor.accept(io)) }
    }
}

impl<Io: IoStream> IoStream for TlsStream<Io> {
    #[inline]
    fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
        let _ = self.get_mut().shutdown();
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.get_mut().get_mut().set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().get_mut().set_linger(dur)
    }
}

impl<Io: IoStream> Future for Accept<Io> {
    type Item = TlsStream<Io>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner.take().expect("cannot poll MidHandshake twice") {
            Ok(stream) => Ok(TlsStream { inner: stream }.into()),
            Err(HandshakeError::Failure(e)) => Err(io::Error::new(io::ErrorKind::Other, e)),
            Err(HandshakeError::WouldBlock(s)) => {
                match s.handshake() {
                    Ok(stream) => Ok(TlsStream { inner: stream }.into()),
                    Err(HandshakeError::Failure(e)) => 
                        Err(io::Error::new(io::ErrorKind::Other, e)),
                    Err(HandshakeError::WouldBlock(s)) => {
                        self.inner = Some(Err(HandshakeError::WouldBlock(s)));
                        Ok(Async::NotReady)
                    }
                }
            }
        }
    }
}

impl<S> TlsStream<S> {
    /// Get access to the internal `native_tls::TlsStream` stream which also
    /// transitively allows access to `S`.
    pub fn get_ref(&self) -> &native_tls::TlsStream<S> {
        &self.inner
    }

    /// Get mutable access to the internal `native_tls::TlsStream` stream which
    /// also transitively allows mutable access to `S`.
    pub fn get_mut(&mut self) -> &mut native_tls::TlsStream<S> {
        &mut self.inner
    }
}

impl<S: io::Read + io::Write> io::Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<S: io::Read + io::Write> io::Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}


impl<S: AsyncRead + AsyncWrite> AsyncRead for TlsStream<S> {
}

impl<S: AsyncRead + AsyncWrite> AsyncWrite for TlsStream<S> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match self.inner.shutdown() {
            Ok(_) => (),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => (),
            Err(e) => return Err(e),
        }
        self.inner.get_mut().shutdown()
    }
}
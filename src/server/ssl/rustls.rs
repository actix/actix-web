use std::net::Shutdown;
use std::{io, time};

use actix_net::ssl; //::RustlsAcceptor;
use rustls::{ClientSession, ServerConfig, ServerSession};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsStream;

use server::{IoStream, ServerFlags};

/// Support `SSL` connections via rustls package
///
/// `rust-tls` feature enables `RustlsAcceptor` type
pub struct RustlsAcceptor<T> {
    _t: ssl::RustlsAcceptor<T>,
}

impl<T: AsyncRead + AsyncWrite> RustlsAcceptor<T> {
    /// Create `RustlsAcceptor` with custom server flags.
    pub fn with_flags(
        mut config: ServerConfig, flags: ServerFlags,
    ) -> ssl::RustlsAcceptor<T> {
        let mut protos = Vec::new();
        if flags.contains(ServerFlags::HTTP2) {
            protos.push("h2".to_string());
        }
        if flags.contains(ServerFlags::HTTP1) {
            protos.push("http/1.1".to_string());
        }
        if !protos.is_empty() {
            config.set_protocols(&protos);
        }

        ssl::RustlsAcceptor::new(config)
    }
}

impl<Io: IoStream> IoStream for TlsStream<Io, ClientSession> {
    #[inline]
    fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
        let _ = <Self as AsyncWrite>::shutdown(self);
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.get_mut().0.set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().0.set_linger(dur)
    }

    #[inline]
    fn set_keepalive(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().0.set_keepalive(dur)
    }
}

impl<Io: IoStream> IoStream for TlsStream<Io, ServerSession> {
    #[inline]
    fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
        let _ = <Self as AsyncWrite>::shutdown(self);
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.get_mut().0.set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().0.set_linger(dur)
    }

    #[inline]
    fn set_keepalive(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().0.set_keepalive(dur)
    }
}

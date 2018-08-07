use std::net::Shutdown;
use std::sync::Arc;
use std::{io, time};

use rustls::{ClientSession, ServerConfig, ServerSession};
use tokio_io::AsyncWrite;
use tokio_rustls::{AcceptAsync, ServerConfigExt, TlsStream};

use server::{AcceptorService, IoStream, ServerFlags};

#[derive(Clone)]
/// Support `SSL` connections via rustls package
///
/// `rust-tls` feature enables `RustlsAcceptor` type
pub struct RustlsAcceptor {
    config: Arc<ServerConfig>,
}

impl RustlsAcceptor {
    /// Create `OpensslAcceptor` with enabled `HTTP/2` and `HTTP1.1` support.
    pub fn new(config: ServerConfig) -> Self {
        RustlsAcceptor::with_flags(config, ServerFlags::HTTP1 | ServerFlags::HTTP2)
    }

    /// Create `OpensslAcceptor` with custom server flags.
    pub fn with_flags(mut config: ServerConfig, flags: ServerFlags) -> Self {
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

        RustlsAcceptor {
            config: Arc::new(config),
        }
    }
}

impl<Io: IoStream> AcceptorService<Io> for RustlsAcceptor {
    type Accepted = TlsStream<Io, ServerSession>;
    type Future = AcceptAsync<Io>;

    fn scheme(&self) -> &'static str {
        "https"
    }

    fn accept(&self, io: Io) -> Self::Future {
        ServerConfigExt::accept_async(&self.config, io)
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
}

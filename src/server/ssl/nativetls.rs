use std::net::Shutdown;
use std::{io, time};

use futures::{Future, Poll};
use native_tls::TlsAcceptor;
use tokio_tls::{AcceptAsync, TlsAcceptorExt, TlsStream};

use server::{AcceptorService, IoStream};

#[derive(Clone)]
/// Support `SSL` connections via native-tls package
///
/// `tls` feature enables `NativeTlsAcceptor` type
pub struct NativeTlsAcceptor {
    acceptor: TlsAcceptor,
}

impl NativeTlsAcceptor {
    /// Create `NativeTlsAcceptor` instance
    pub fn new(acceptor: TlsAcceptor) -> Self {
        NativeTlsAcceptor { acceptor }
    }
}

pub struct AcceptorFut<Io>(AcceptAsync<Io>);

impl<Io: IoStream> Future for AcceptorFut<Io> {
    type Item = TlsStream<Io>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0
            .poll()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

impl<Io: IoStream> AcceptorService<Io> for NativeTlsAcceptor {
    type Accepted = TlsStream<Io>;
    type Future = AcceptorFut<Io>;

    fn scheme(&self) -> &'static str {
        "https"
    }

    fn accept(&self, io: Io) -> Self::Future {
        AcceptorFut(TlsAcceptorExt::accept_async(&self.acceptor, io))
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

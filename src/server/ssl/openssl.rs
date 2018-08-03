use std::net::Shutdown;
use std::{io, time};

use futures::{Future, Poll};
use openssl::ssl::{AlpnError, SslAcceptor, SslAcceptorBuilder};
use tokio_openssl::{AcceptAsync, SslAcceptorExt, SslStream};

use server::{AcceptorService, IoStream, ServerFlags};

#[derive(Clone)]
/// Support `SSL` connections via openssl package
///
/// `alpn` feature enables `OpensslAcceptor` type
pub struct OpensslAcceptor {
    acceptor: SslAcceptor,
}

impl OpensslAcceptor {
    /// Create `OpensslAcceptor` with enabled `HTTP/2` and `HTTP1.1` support.
    pub fn new(builder: SslAcceptorBuilder) -> io::Result<Self> {
        OpensslAcceptor::with_flags(builder, ServerFlags::HTTP1 | ServerFlags::HTTP2)
    }

    /// Create `OpensslAcceptor` with custom server flags.
    pub fn with_flags(
        mut builder: SslAcceptorBuilder, flags: ServerFlags,
    ) -> io::Result<Self> {
        let mut protos = Vec::new();
        if flags.contains(ServerFlags::HTTP1) {
            protos.extend(b"\x08http/1.1");
        }
        if flags.contains(ServerFlags::HTTP2) {
            protos.extend(b"\x02h2");
            builder.set_alpn_select_callback(|_, protos| {
                const H2: &[u8] = b"\x02h2";
                if protos.windows(3).any(|window| window == H2) {
                    Ok(b"h2")
                } else {
                    Err(AlpnError::NOACK)
                }
            });
        }

        if !protos.is_empty() {
            builder.set_alpn_protos(&protos)?;
        }

        Ok(OpensslAcceptor {
            acceptor: builder.build(),
        })
    }
}

pub struct AcceptorFut<Io>(AcceptAsync<Io>);

impl<Io: IoStream> Future for AcceptorFut<Io> {
    type Item = SslStream<Io>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0
            .poll()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

impl<Io: IoStream> AcceptorService<Io> for OpensslAcceptor {
    type Accepted = SslStream<Io>;
    type Future = AcceptorFut<Io>;

    fn scheme(&self) -> &'static str {
        "https"
    }

    fn accept(&self, io: Io) -> Self::Future {
        AcceptorFut(SslAcceptorExt::accept_async(&self.acceptor, io))
    }
}

impl<T: IoStream> IoStream for SslStream<T> {
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

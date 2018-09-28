use std::net::Shutdown;
use std::{io, time};

use actix_net::ssl;
use openssl::ssl::{AlpnError, SslAcceptor, SslAcceptorBuilder};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslStream;

use server::{IoStream, ServerFlags};

/// Support `SSL` connections via openssl package
///
/// `ssl` feature enables `OpensslAcceptor` type
pub struct OpensslAcceptor<T> {
    _t: ssl::OpensslAcceptor<T>,
}

impl<T: AsyncRead + AsyncWrite> OpensslAcceptor<T> {
    /// Create `OpensslAcceptor` with enabled `HTTP/2` and `HTTP1.1` support.
    pub fn new(builder: SslAcceptorBuilder) -> io::Result<ssl::OpensslAcceptor<T>> {
        OpensslAcceptor::with_flags(builder, ServerFlags::HTTP1 | ServerFlags::HTTP2)
    }

    /// Create `OpensslAcceptor` with custom server flags.
    pub fn with_flags(
        mut builder: SslAcceptorBuilder, flags: ServerFlags,
    ) -> io::Result<ssl::OpensslAcceptor<T>> {
        let acceptor = openssl_acceptor_with_flags(builder, flags)?;

        Ok(ssl::OpensslAcceptor::new(acceptor))
    }
}

/// Configure `SslAcceptorBuilder` with custom server flags.
pub fn openssl_acceptor_with_flags(
    mut builder: SslAcceptorBuilder, flags: ServerFlags,
) -> io::Result<SslAcceptor> {
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

    Ok(builder.build())
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

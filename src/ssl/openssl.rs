use std::marker::PhantomData;
use std::net::Shutdown;
use std::{io, time};

use futures::{future, future::FutureResult, Async, Future, Poll};
use openssl::ssl::{AlpnError, SslAcceptor, SslAcceptorBuilder};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{AcceptAsync, SslAcceptorExt, SslStream};
use tokio_tcp::TcpStream;
use tower_service::{NewService, Service};

use {IntoNewService, IoStream};

/// Support `SSL` connections via openssl package
///
/// `alpn` feature enables `OpensslAcceptor` type
pub struct OpensslService<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
}

impl<T> OpensslService<T> {
    /// Create default `OpensslService`
    pub fn new(builder: SslAcceptorBuilder) -> Self {
        OpensslService {
            acceptor: builder.build(),
            io: PhantomData,
        }
    }

    /// Create `OpensslWith` with `HTTP1.1` and `HTTP2`.
    pub fn for_http(mut builder: SslAcceptorBuilder) -> io::Result<Self> {
        let protos = b"\x08http/1.1\x02h2";

        builder.set_alpn_select_callback(|_, protos| {
            const H2: &[u8] = b"\x02h2";
            if protos.windows(3).any(|window| window == H2) {
                Ok(b"h2")
            } else {
                Err(AlpnError::NOACK)
            }
        });
        builder.set_alpn_protos(&protos[..])?;

        Ok(OpensslService {
            acceptor: builder.build(),
            io: PhantomData,
        })
    }
}
impl<T: AsyncRead + AsyncWrite> Clone for OpensslService<T> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> NewService for OpensslService<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = io::Error;
    type Service = OpensslAcceptor<T>;
    type InitError = io::Error;
    type Future = FutureResult<Self::Service, io::Error>;

    fn new_service(&self) -> Self::Future {
        future::ok(OpensslAcceptor {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        })
    }
}

pub struct OpensslAcceptor<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite> Service for OpensslAcceptor<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = io::Error;
    type Future = AcceptorFuture<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        AcceptorFuture(SslAcceptorExt::accept_async(&self.acceptor, req))
    }
}

pub struct AcceptorFuture<T>(AcceptAsync<T>);

impl<T: AsyncRead + AsyncWrite> Future for AcceptorFuture<T> {
    type Item = SslStream<T>;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0
            .poll()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

// impl<T: IoStream> IoStream for SslStream<T> {
//     #[inline]
//     fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
//         let _ = self.get_mut().shutdown();
//         Ok(())
//     }

//     #[inline]
//     fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
//         self.get_mut().get_mut().set_nodelay(nodelay)
//     }

//     #[inline]
//     fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
//         self.get_mut().get_mut().set_linger(dur)
//     }
// }

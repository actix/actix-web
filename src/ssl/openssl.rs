use std::io;
use std::marker::PhantomData;

use futures::{future, future::FutureResult, Async, Future, Poll};
use openssl::ssl::{AlpnError, Error, SslAcceptor, SslAcceptorBuilder, SslConnector};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{AcceptAsync, ConnectAsync, SslAcceptorExt, SslConnectorExt, SslStream};

use connector::ConnectionInfo;
use {NewService, Service};

/// Support `SSL` connections via openssl package
///
/// `ssl` feature enables `OpensslAcceptor` type
pub struct OpensslAcceptor<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
}

impl<T> OpensslAcceptor<T> {
    /// Create default `OpensslAcceptor`
    pub fn new(builder: SslAcceptorBuilder) -> Self {
        OpensslAcceptor {
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

        Ok(OpensslAcceptor {
            acceptor: builder.build(),
            io: PhantomData,
        })
    }
}
impl<T: AsyncRead + AsyncWrite> Clone for OpensslAcceptor<T> {
    fn clone(&self) -> Self {
        Self {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> NewService for OpensslAcceptor<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = Error;
    type Service = OpensslAcceptorService<T>;
    type InitError = io::Error;
    type Future = FutureResult<Self::Service, io::Error>;

    fn new_service(&self) -> Self::Future {
        future::ok(OpensslAcceptorService {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        })
    }
}

pub struct OpensslAcceptorService<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite> Service for OpensslAcceptorService<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = Error;
    type Future = AcceptAsync<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        SslAcceptorExt::accept_async(&self.acceptor, req)
    }
}

/// Openssl connector factory
pub struct OpensslConnector<T> {
    connector: SslConnector,
    io: PhantomData<T>,
}

impl<T> OpensslConnector<T> {
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnector {
            connector,
            io: PhantomData,
        }
    }
}

impl<T> Clone for OpensslConnector<T> {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
            io: PhantomData,
        }
    }
}

impl<T: AsyncRead + AsyncWrite> NewService for OpensslConnector<T> {
    type Request = (ConnectionInfo, T);
    type Response = (ConnectionInfo, SslStream<T>);
    type Error = Error;
    type Service = OpensslConnectorService<T>;
    type InitError = io::Error;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        future::ok(OpensslConnectorService {
            connector: self.connector.clone(),
            io: PhantomData,
        })
    }
}

pub struct OpensslConnectorService<T> {
    connector: SslConnector,
    io: PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite> Service for OpensslConnectorService<T> {
    type Request = (ConnectionInfo, T);
    type Response = (ConnectionInfo, SslStream<T>);
    type Error = Error;
    type Future = ConnectAsyncExt<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (info, stream): Self::Request) -> Self::Future {
        ConnectAsyncExt {
            fut: SslConnectorExt::connect_async(&self.connector, &info.host, stream),
            host: Some(info),
        }
    }
}

pub struct ConnectAsyncExt<T> {
    fut: ConnectAsync<T>,
    host: Option<ConnectionInfo>,
}

impl<T> Future for ConnectAsyncExt<T>
where
    T: AsyncRead + AsyncWrite,
{
    type Item = (ConnectionInfo, SslStream<T>);
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(stream) => Ok(Async::Ready((self.host.take().unwrap(), stream))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

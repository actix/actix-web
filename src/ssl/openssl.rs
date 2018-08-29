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
pub struct OpensslConnector<T, Io> {
    connector: SslConnector,
    t: PhantomData<T>,
    io: PhantomData<Io>,
}

impl<T, Io> OpensslConnector<T, Io> {
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnector {
            connector,
            t: PhantomData,
            io: PhantomData,
        }
    }
}

impl<T, Io> Clone for OpensslConnector<T, Io> {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
            t: PhantomData,
            io: PhantomData,
        }
    }
}

impl<T, Io: AsyncRead + AsyncWrite> NewService for OpensslConnector<T, Io> {
    type Request = (T, ConnectionInfo, Io);
    type Response = (T, ConnectionInfo, SslStream<Io>);
    type Error = Error;
    type Service = OpensslConnectorService<T, Io>;
    type InitError = io::Error;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        future::ok(OpensslConnectorService {
            connector: self.connector.clone(),
            t: PhantomData,
            io: PhantomData,
        })
    }
}

pub struct OpensslConnectorService<T, Io> {
    connector: SslConnector,
    t: PhantomData<T>,
    io: PhantomData<Io>,
}

impl<T, Io: AsyncRead + AsyncWrite> Service for OpensslConnectorService<T, Io> {
    type Request = (T, ConnectionInfo, Io);
    type Response = (T, ConnectionInfo, SslStream<Io>);
    type Error = Error;
    type Future = ConnectAsyncExt<T, Io>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, (req, info, stream): Self::Request) -> Self::Future {
        ConnectAsyncExt {
            fut: SslConnectorExt::connect_async(&self.connector, &info.host, stream),
            req: Some(req),
            host: Some(info),
        }
    }
}

pub struct ConnectAsyncExt<T, Io> {
    fut: ConnectAsync<Io>,
    req: Option<T>,
    host: Option<ConnectionInfo>,
}

impl<T, Io> Future for ConnectAsyncExt<T, Io>
where
    Io: AsyncRead + AsyncWrite,
{
    type Item = (T, ConnectionInfo, SslStream<Io>);
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll()? {
            Async::Ready(stream) => Ok(Async::Ready((
                self.req.take().unwrap(),
                self.host.take().unwrap(),
                stream,
            ))),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

use std::cell::Cell;
use std::io;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use futures::task::AtomicTask;
use futures::{future::ok, future::FutureResult, Async, Future, Poll};
use openssl::ssl::{AlpnError, Error, SslAcceptor, SslAcceptorBuilder, SslConnector};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{AcceptAsync, ConnectAsync, SslAcceptorExt, SslConnectorExt, SslStream};

use super::MAX_CONN;
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
        ok(OpensslAcceptorService {
            acceptor: self.acceptor.clone(),
            io: PhantomData,
            inner: Rc::new(Inner::default()),
        })
    }
}

pub struct OpensslAcceptorService<T> {
    acceptor: SslAcceptor,
    io: PhantomData<T>,
    inner: Rc<Inner>,
}

impl<T: AsyncRead + AsyncWrite> Service for OpensslAcceptorService<T> {
    type Request = T;
    type Response = SslStream<T>;
    type Error = Error;
    type Future = OpensslAcceptorServiceFut<T>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        if self.inner.check() {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        self.inner.inc();

        OpensslAcceptorServiceFut {
            inner: self.inner.clone(),
            fut: SslAcceptorExt::accept_async(&self.acceptor, req),
        }
    }
}

struct Inner {
    maxconn: usize,
    count: Cell<usize>,
    task: AtomicTask,
}

impl Default for Inner {
    fn default() -> Inner {
        Inner {
            maxconn: MAX_CONN.load(Ordering::Relaxed),
            count: Cell::new(0),
            task: AtomicTask::new(),
        }
    }
}

impl Inner {
    fn inc(&self) {
        let num = self.count.get() + 1;
        self.count.set(num);
        if num == self.maxconn {
            self.task.register();
        }
    }

    fn dec(&self) {
        let num = self.count.get();
        self.count.set(num - 1);
        if num == self.maxconn {
            self.task.notify();
        }
    }

    fn check(&self) -> bool {
        self.count.get() < self.maxconn
    }
}

pub struct OpensslAcceptorServiceFut<T>
where
    T: AsyncRead + AsyncWrite,
{
    fut: AcceptAsync<T>,
    inner: Rc<Inner>,
}

impl<T: AsyncRead + AsyncWrite> Future for OpensslAcceptorServiceFut<T> {
    type Item = SslStream<T>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let res = self.fut.poll();

        if let Ok(Async::NotReady) = res {
            Ok(Async::NotReady)
        } else {
            self.inner.dec();
            res
        }
    }
}

/// Openssl connector factory
pub struct OpensslConnector<T, Io, E> {
    connector: SslConnector,
    t: PhantomData<T>,
    io: PhantomData<Io>,
    _e: PhantomData<E>,
}

impl<T, Io, E> OpensslConnector<T, Io, E> {
    pub fn new(connector: SslConnector) -> Self {
        OpensslConnector {
            connector,
            t: PhantomData,
            io: PhantomData,
            _e: PhantomData,
        }
    }
}

impl<T, Io: AsyncRead + AsyncWrite> OpensslConnector<T, Io, ()> {
    pub fn service(
        connector: SslConnector,
    ) -> impl Service<
        Request = (T, ConnectionInfo, Io),
        Response = (T, ConnectionInfo, SslStream<Io>),
        Error = Error,
    > {
        OpensslConnectorService {
            connector: connector,
            t: PhantomData,
            io: PhantomData,
        }
    }
}

impl<T, Io, E> Clone for OpensslConnector<T, Io, E> {
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
            t: PhantomData,
            io: PhantomData,
            _e: PhantomData,
        }
    }
}

impl<T, Io: AsyncRead + AsyncWrite, E> NewService for OpensslConnector<T, Io, E> {
    type Request = (T, ConnectionInfo, Io);
    type Response = (T, ConnectionInfo, SslStream<Io>);
    type Error = Error;
    type Service = OpensslConnectorService<T, Io>;
    type InitError = E;
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        ok(OpensslConnectorService {
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

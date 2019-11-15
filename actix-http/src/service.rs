use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io, net, rc};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_server_config::{
    Io as ServerIo, IoStream, Protocol, ServerConfig as SrvConfig,
};
use actix_service::{IntoServiceFactory, Service, ServiceFactory};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{ready, Future};
use h2::server::{self, Handshake};

use crate::body::MessageBody;
use crate::builder::HttpServiceBuilder;
use crate::cloneable::CloneableService;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, Error};
use crate::helpers::DataFactory;
use crate::request::Request;
use crate::response::Response;
use crate::{h1, h2::Dispatcher};

/// `ServiceFactory` HTTP1.1/HTTP2 transport implementation
pub struct HttpService<T, P, S, B, X = h1::ExpectHandler, U = h1::UpgradeHandler<T>> {
    srv: S,
    cfg: ServiceConfig,
    expect: X,
    upgrade: Option<U>,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, S, B> HttpService<T, (), S, B>
where
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error> + Unpin + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + Unpin + 'static,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin + 'static,
    B: MessageBody + 'static,
{
    /// Create builder for `HttpService` instance.
    pub fn build() -> HttpServiceBuilder<T, S> {
        HttpServiceBuilder::new()
    }
}

impl<T, P, S, B> HttpService<T, P, S, B>
where
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error> + Unpin + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + Unpin + 'static,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin + 'static,
    B: MessageBody + 'static,
    P: Unpin,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoServiceFactory<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        HttpService {
            cfg,
            srv: service.into_factory(),
            expect: h1::ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub(crate) fn with_config<F: IntoServiceFactory<S>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        HttpService {
            cfg,
            srv: service.into_factory(),
            expect: h1::ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X, U> HttpService<T, P, S, B, X, U>
where
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error> + Unpin + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + Unpin + 'static,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin + 'static,
    B: MessageBody,
    P: Unpin,
{
    /// Provide service for `EXPECT: 100-Continue` support.
    ///
    /// Service get called with request that contains `EXPECT` header.
    /// Service must return request in case of success, in that case
    /// request will be forwarded to main service.
    pub fn expect<X1>(self, expect: X1) -> HttpService<T, P, S, B, X1, U>
    where
        X1: ServiceFactory<Config = SrvConfig, Request = Request, Response = Request>,
        X1::Error: Into<Error>,
        X1::InitError: fmt::Debug,
        X1::Future: Unpin,
        X1::Service: Unpin,
        <X1::Service as Service>::Future: Unpin + 'static,
    {
        HttpService {
            expect,
            cfg: self.cfg,
            srv: self.srv,
            upgrade: self.upgrade,
            on_connect: self.on_connect,
            _t: PhantomData,
        }
    }

    /// Provide service for custom `Connection: UPGRADE` support.
    ///
    /// If service is provided then normal requests handling get halted
    /// and this service get called with original request and framed object.
    pub fn upgrade<U1>(self, upgrade: Option<U1>) -> HttpService<T, P, S, B, X, U1>
    where
        U1: ServiceFactory<
            Config = SrvConfig,
            Request = (Request, Framed<T, h1::Codec>),
            Response = (),
        >,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
        U1::Future: Unpin,
        U1::Service: Unpin,
        <U1::Service as Service>::Future: Unpin + 'static,
    {
        HttpService {
            upgrade,
            cfg: self.cfg,
            srv: self.srv,
            expect: self.expect,
            on_connect: self.on_connect,
            _t: PhantomData,
        }
    }

    /// Set on connect callback.
    pub(crate) fn on_connect(
        mut self,
        f: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    ) -> Self {
        self.on_connect = f;
        self
    }
}

impl<T, P, S, B, X, U> ServiceFactory for HttpService<T, P, S, B, X, U>
where
    T: IoStream + Unpin,
    S: ServiceFactory<Config = SrvConfig, Request = Request>,
    S::Service: Unpin,
    S::Error: Into<Error> + Unpin + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + Unpin + 'static,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin + 'static,
    B: MessageBody + 'static,
    X: ServiceFactory<Config = SrvConfig, Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    X::Future: Unpin,
    X::Service: Unpin,
    <X::Service as Service>::Future: Unpin + 'static,
    U: ServiceFactory<
        Config = SrvConfig,
        Request = (Request, Framed<T, h1::Codec>),
        Response = (),
    >,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
    U::Future: Unpin,
    U::Service: Unpin,
    <U::Service as Service>::Future: Unpin + 'static,
    P: Unpin,
{
    type Config = SrvConfig;
    type Request = ServerIo<T, P>;
    type Response = ();
    type Error = DispatchError;
    type InitError = ();
    type Service = HttpServiceHandler<T, P, S::Service, B, X::Service, U::Service>;
    type Future = HttpServiceResponse<T, P, S, B, X, U>;

    fn new_service(&self, cfg: &SrvConfig) -> Self::Future {
        HttpServiceResponse {
            fut: self.srv.new_service(cfg),
            fut_ex: Some(self.expect.new_service(cfg)),
            fut_upg: self.upgrade.as_ref().map(|f| f.new_service(cfg)),
            expect: None,
            upgrade: None,
            on_connect: self.on_connect.clone(),
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct HttpServiceResponse<
    T,
    P,
    S: ServiceFactory,
    B,
    X: ServiceFactory,
    U: ServiceFactory,
> {
    fut: S::Future,
    fut_ex: Option<X::Future>,
    fut_upg: Option<U::Future>,
    expect: Option<X::Service>,
    upgrade: Option<U::Service>,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B, X, U> Future for HttpServiceResponse<T, P, S, B, X, U>
where
    T: IoStream,
    S: ServiceFactory<Request = Request>,
    S::Error: Into<Error> + Unpin + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + Unpin + 'static,
    S::Future: Unpin,
    S::Service: Unpin,
    <S::Service as Service>::Future: Unpin + 'static,
    B: MessageBody + 'static,
    X: ServiceFactory<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    X::Future: Unpin,
    X::Service: Unpin,
    <X::Service as Service>::Future: Unpin + 'static,
    U: ServiceFactory<Request = (Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
    U::Future: Unpin,
    U::Service: Unpin,
    <U::Service as Service>::Future: Unpin + 'static,
    P: Unpin,
{
    type Output =
        Result<HttpServiceHandler<T, P, S::Service, B, X::Service, U::Service>, ()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();

        if let Some(ref mut fut) = this.fut_ex {
            let expect = ready!(Pin::new(fut)
                .poll(cx)
                .map_err(|e| log::error!("Init http service error: {:?}", e)))?;
            this.expect = Some(expect);
            this.fut_ex.take();
        }

        if let Some(ref mut fut) = this.fut_upg {
            let upgrade = ready!(Pin::new(fut)
                .poll(cx)
                .map_err(|e| log::error!("Init http service error: {:?}", e)))?;
            this.upgrade = Some(upgrade);
            this.fut_ex.take();
        }

        let result = ready!(Pin::new(&mut this.fut)
            .poll(cx)
            .map_err(|e| log::error!("Init http service error: {:?}", e)));
        Poll::Ready(result.map(|service| {
            HttpServiceHandler::new(
                this.cfg.take().unwrap(),
                service,
                this.expect.take().unwrap(),
                this.upgrade.take(),
                this.on_connect.clone(),
            )
        }))
    }
}

/// `Service` implementation for http transport
pub struct HttpServiceHandler<T, P, S, B, X, U> {
    srv: CloneableService<S>,
    expect: CloneableService<X>,
    upgrade: Option<CloneableService<U>>,
    cfg: ServiceConfig,
    on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    _t: PhantomData<(T, P, B, X)>,
}

impl<T, P, S, B, X, U> HttpServiceHandler<T, P, S, B, X, U>
where
    S: Service<Request = Request> + Unpin,
    S::Error: Into<Error> + Unpin + 'static,
    S::Future: 'static,
    S::Response: Into<Response<B>> + Unpin + 'static,
    S::Future: Unpin,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request> + Unpin,
    X::Future: Unpin,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()> + Unpin,
    U::Future: Unpin,
    U::Error: fmt::Display,
    P: Unpin,
{
    fn new(
        cfg: ServiceConfig,
        srv: S,
        expect: X,
        upgrade: Option<U>,
        on_connect: Option<rc::Rc<dyn Fn(&T) -> Box<dyn DataFactory>>>,
    ) -> HttpServiceHandler<T, P, S, B, X, U> {
        HttpServiceHandler {
            cfg,
            on_connect,
            srv: CloneableService::new(srv),
            expect: CloneableService::new(expect),
            upgrade: upgrade.map(CloneableService::new),
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X, U> Service for HttpServiceHandler<T, P, S, B, X, U>
where
    T: IoStream + Unpin,
    S: Service<Request = Request> + Unpin,
    S::Error: Into<Error> + Unpin + 'static,
    S::Future: Unpin + 'static,
    S::Response: Into<Response<B>> + Unpin + 'static,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request> + Unpin,
    X::Error: Into<Error>,
    X::Future: Unpin,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()> + Unpin,
    U::Error: fmt::Display,
    U::Future: Unpin,
    P: Unpin,
{
    type Request = ServerIo<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = HttpServiceHandlerResponse<T, S, B, X, U>;

    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let ready = self
            .expect
            .poll_ready(cx)
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready();

        let ready = self
            .srv
            .poll_ready(cx)
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready()
            && ready;

        if ready {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let (io, _, proto) = req.into_parts();

        let on_connect = if let Some(ref on_connect) = self.on_connect {
            Some(on_connect(&io))
        } else {
            None
        };

        match proto {
            Protocol::Http2 => {
                let peer_addr = io.peer_addr();
                let io = Io {
                    inner: io,
                    unread: None,
                };
                HttpServiceHandlerResponse {
                    state: State::Handshake(Some((
                        server::handshake(io),
                        self.cfg.clone(),
                        self.srv.clone(),
                        peer_addr,
                        on_connect,
                    ))),
                }
            }
            Protocol::Http10 | Protocol::Http11 => HttpServiceHandlerResponse {
                state: State::H1(h1::Dispatcher::new(
                    io,
                    self.cfg.clone(),
                    self.srv.clone(),
                    self.expect.clone(),
                    self.upgrade.clone(),
                    on_connect,
                )),
            },
            _ => HttpServiceHandlerResponse {
                state: State::Unknown(Some((
                    io,
                    BytesMut::with_capacity(14),
                    self.cfg.clone(),
                    self.srv.clone(),
                    self.expect.clone(),
                    self.upgrade.clone(),
                    on_connect,
                ))),
            },
        }
    }
}

enum State<T, S, B, X, U>
where
    S: Service<Request = Request> + Unpin,
    S::Future: Unpin + 'static,
    S::Error: Into<Error>,
    T: IoStream + Unpin,
    B: MessageBody,
    X: Service<Request = Request, Response = Request> + Unpin,
    X::Error: Into<Error>,
    X::Future: Unpin,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()> + Unpin,
    U::Error: fmt::Display,
    U::Future: Unpin,
{
    H1(h1::Dispatcher<T, S, B, X, U>),
    H2(Dispatcher<Io<T>, S, B>),
    Unknown(
        Option<(
            T,
            BytesMut,
            ServiceConfig,
            CloneableService<S>,
            CloneableService<X>,
            Option<CloneableService<U>>,
            Option<Box<dyn DataFactory>>,
        )>,
    ),
    Handshake(
        Option<(
            Handshake<Io<T>, Bytes>,
            ServiceConfig,
            CloneableService<S>,
            Option<net::SocketAddr>,
            Option<Box<dyn DataFactory>>,
        )>,
    ),
}

pub struct HttpServiceHandlerResponse<T, S, B, X, U>
where
    T: IoStream + Unpin,
    S: Service<Request = Request> + Unpin,
    S::Error: Into<Error> + Unpin + 'static,
    S::Future: Unpin + 'static,
    S::Response: Into<Response<B>> + Unpin + 'static,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request> + Unpin,
    X::Error: Into<Error>,
    X::Future: Unpin,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()> + Unpin,
    U::Error: fmt::Display,
    U::Future: Unpin,
{
    state: State<T, S, B, X, U>,
}

const HTTP2_PREFACE: [u8; 14] = *b"PRI * HTTP/2.0";

impl<T, S, B, X, U> Future for HttpServiceHandlerResponse<T, S, B, X, U>
where
    T: IoStream + Unpin,
    S: Service<Request = Request> + Unpin,
    S::Error: Into<Error> + Unpin + 'static,
    S::Future: Unpin + 'static,
    S::Response: Into<Response<B>> + Unpin + 'static,
    B: MessageBody,
    X: Service<Request = Request, Response = Request> + Unpin,
    X::Future: Unpin,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()> + Unpin,
    U::Future: Unpin,
    U::Error: fmt::Display,
{
    type Output = Result<(), DispatchError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match self.state {
            State::H1(ref mut disp) => Pin::new(disp).poll(cx),
            State::H2(ref mut disp) => Pin::new(disp).poll(cx),
            State::Unknown(ref mut data) => {
                if let Some(ref mut item) = data {
                    loop {
                        // Safety - we only write to the returned slice.
                        let b = unsafe { item.1.bytes_mut() };
                        let n = ready!(Pin::new(&mut item.0).poll_read(cx, b))?;
                        if n == 0 {
                            return Poll::Ready(Ok(()));
                        }
                        // Safety - we know that 'n' bytes have
                        // been initialized via the contract of
                        // 'poll_read'
                        unsafe { item.1.advance_mut(n) };
                        if item.1.len() >= HTTP2_PREFACE.len() {
                            break;
                        }
                    }
                } else {
                    panic!()
                }
                let (io, buf, cfg, srv, expect, upgrade, on_connect) =
                    data.take().unwrap();
                if buf[..14] == HTTP2_PREFACE[..] {
                    let peer_addr = io.peer_addr();
                    let io = Io {
                        inner: io,
                        unread: Some(buf),
                    };
                    self.state = State::Handshake(Some((
                        server::handshake(io),
                        cfg,
                        srv,
                        peer_addr,
                        on_connect,
                    )));
                } else {
                    self.state = State::H1(h1::Dispatcher::with_timeout(
                        io,
                        h1::Codec::new(cfg.clone()),
                        cfg,
                        buf,
                        None,
                        srv,
                        expect,
                        upgrade,
                        on_connect,
                    ))
                }
                self.poll(cx)
            }
            State::Handshake(ref mut data) => {
                let conn = if let Some(ref mut item) = data {
                    match Pin::new(&mut item.0).poll(cx) {
                        Poll::Ready(Ok(conn)) => conn,
                        Poll::Ready(Err(err)) => {
                            trace!("H2 handshake error: {}", err);
                            return Poll::Ready(Err(err.into()));
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                } else {
                    panic!()
                };
                let (_, cfg, srv, peer_addr, on_connect) = data.take().unwrap();
                self.state = State::H2(Dispatcher::new(
                    srv, conn, on_connect, cfg, None, peer_addr,
                ));
                self.poll(cx)
            }
        }
    }
}

/// Wrapper for `AsyncRead + AsyncWrite` types
struct Io<T> {
    unread: Option<BytesMut>,
    inner: T,
}

impl<T> Unpin for Io<T> {}

impl<T: io::Read> io::Read for Io<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(mut bytes) = self.unread.take() {
            let size = std::cmp::min(buf.len(), bytes.len());
            buf[..size].copy_from_slice(&bytes[..size]);
            if bytes.len() > size {
                bytes.split_to(size);
                self.unread = Some(bytes);
            }
            Ok(size)
        } else {
            self.inner.read(buf)
        }
    }
}

impl<T: io::Write> io::Write for Io<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Io<T> {
    // unsafe fn initializer(&self) -> io::Initializer {
    //     self.get_mut().inner.initializer()
    // }

    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.inner.prepare_uninitialized_buffer(buf)
    }

    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }

    // fn poll_read_vectored(
    //     self: Pin<&mut Self>,
    //     cx: &mut Context<'_>,
    //     bufs: &mut [io::IoSliceMut<'_>],
    // ) -> Poll<io::Result<usize>> {
    //     self.get_mut().inner.poll_read_vectored(cx, bufs)
    // }
}

impl<T: AsyncWrite + Unpin> tokio_io::AsyncWrite for Io<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    // fn poll_write_vectored(
    //     self: Pin<&mut Self>,
    //     cx: &mut Context<'_>,
    //     bufs: &[io::IoSlice<'_>],
    // ) -> Poll<io::Result<usize>> {
    //     self.get_mut().inner.poll_write_vectored(cx, bufs)
    // }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<T: IoStream> actix_server_config::IoStream for Io<T> {
    #[inline]
    fn peer_addr(&self) -> Option<net::SocketAddr> {
        self.inner.peer_addr()
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.inner.set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<std::time::Duration>) -> io::Result<()> {
        self.inner.set_linger(dur)
    }

    #[inline]
    fn set_keepalive(&mut self, dur: Option<std::time::Duration>) -> io::Result<()> {
        self.inner.set_keepalive(dur)
    }
}

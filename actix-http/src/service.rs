use std::marker::PhantomData;
use std::{fmt, io, net, rc};

use actix_codec::{AsyncRead, AsyncWrite, Framed};
use actix_server_config::{
    Io as ServerIo, IoStream, Protocol, ServerConfig as SrvConfig,
};
use actix_service::{IntoNewService, NewService, Service};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{try_ready, Async, Future, IntoFuture, Poll};
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

/// `NewService` HTTP1.1/HTTP2 transport implementation
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
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create builder for `HttpService` instance.
    pub fn build() -> HttpServiceBuilder<T, S> {
        HttpServiceBuilder::new()
    }
}

impl<T, P, S, B> HttpService<T, P, S, B>
where
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        HttpService {
            cfg,
            srv: service.into_new_service(),
            expect: h1::ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub(crate) fn with_config<F: IntoNewService<S>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        HttpService {
            cfg,
            srv: service.into_new_service(),
            expect: h1::ExpectHandler,
            upgrade: None,
            on_connect: None,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X, U> HttpService<T, P, S, B, X, U>
where
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    /// Provide service for `EXPECT: 100-Continue` support.
    ///
    /// Service get called with request that contains `EXPECT` header.
    /// Service must return request in case of success, in that case
    /// request will be forwarded to main service.
    pub fn expect<X1>(self, expect: X1) -> HttpService<T, P, S, B, X1, U>
    where
        X1: NewService<Config = SrvConfig, Request = Request, Response = Request>,
        X1::Error: Into<Error>,
        X1::InitError: fmt::Debug,
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
        U1: NewService<
            Config = SrvConfig,
            Request = (Request, Framed<T, h1::Codec>),
            Response = (),
        >,
        U1::Error: fmt::Display,
        U1::InitError: fmt::Debug,
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

impl<T, P, S, B, X, U> NewService for HttpService<T, P, S, B, X, U>
where
    T: IoStream,
    S: NewService<Config = SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
    X: NewService<Config = SrvConfig, Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    U: NewService<
        Config = SrvConfig,
        Request = (Request, Framed<T, h1::Codec>),
        Response = (),
    >,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
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
            fut: self.srv.new_service(cfg).into_future(),
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
pub struct HttpServiceResponse<T, P, S: NewService, B, X: NewService, U: NewService> {
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
    S: NewService<Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
    U: NewService<Request = (Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
    U::InitError: fmt::Debug,
{
    type Item = HttpServiceHandler<T, P, S::Service, B, X::Service, U::Service>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_ex {
            let expect = try_ready!(fut
                .poll()
                .map_err(|e| log::error!("Init http service error: {:?}", e)));
            self.expect = Some(expect);
            self.fut_ex.take();
        }

        if let Some(ref mut fut) = self.fut_upg {
            let upgrade = try_ready!(fut
                .poll()
                .map_err(|e| log::error!("Init http service error: {:?}", e)));
            self.upgrade = Some(upgrade);
            self.fut_ex.take();
        }

        let service = try_ready!(self
            .fut
            .poll()
            .map_err(|e| log::error!("Init http service error: {:?}", e)));
        Ok(Async::Ready(HttpServiceHandler::new(
            self.cfg.take().unwrap(),
            service,
            self.expect.take().unwrap(),
            self.upgrade.take(),
            self.on_connect.clone(),
        )))
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
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
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
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Request = ServerIo<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = HttpServiceHandlerResponse<T, S, B, X, U>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        let ready = self
            .expect
            .poll_ready()
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready();

        let ready = self
            .srv
            .poll_ready()
            .map_err(|e| {
                let e = e.into();
                log::error!("Http service readiness error: {:?}", e);
                DispatchError::Service(e)
            })?
            .is_ready()
            && ready;

        if ready {
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
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
    S: Service<Request = Request>,
    S::Future: 'static,
    S::Error: Into<Error>,
    T: IoStream,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
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
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    state: State<T, S, B, X, U>,
}

const HTTP2_PREFACE: [u8; 14] = *b"PRI * HTTP/2.0";

impl<T, S, B, X, U> Future for HttpServiceHandlerResponse<T, S, B, X, U>
where
    T: IoStream,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    U: Service<Request = (Request, Framed<T, h1::Codec>), Response = ()>,
    U::Error: fmt::Display,
{
    type Item = ();
    type Error = DispatchError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.state {
            State::H1(ref mut disp) => disp.poll(),
            State::H2(ref mut disp) => disp.poll(),
            State::Unknown(ref mut data) => {
                if let Some(ref mut item) = data {
                    loop {
                        // Safety - we only write to the returned slice.
                        let b = unsafe { item.1.bytes_mut() };
                        let n = try_ready!(item.0.poll_read(b));
                        if n == 0 {
                            return Ok(Async::Ready(()));
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
                self.poll()
            }
            State::Handshake(ref mut data) => {
                let conn = if let Some(ref mut item) = data {
                    match item.0.poll() {
                        Ok(Async::Ready(conn)) => conn,
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Err(err) => {
                            trace!("H2 handshake error: {}", err);
                            return Err(err.into());
                        }
                    }
                } else {
                    panic!()
                };
                let (_, cfg, srv, peer_addr, on_connect) = data.take().unwrap();
                self.state = State::H2(Dispatcher::new(
                    srv, conn, on_connect, cfg, None, peer_addr,
                ));
                self.poll()
            }
        }
    }
}

/// Wrapper for `AsyncRead + AsyncWrite` types
struct Io<T> {
    unread: Option<BytesMut>,
    inner: T,
}

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

impl<T: AsyncRead> AsyncRead for Io<T> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.inner.prepare_uninitialized_buffer(buf)
    }
}

impl<T: AsyncWrite> AsyncWrite for Io<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.inner.shutdown()
    }
    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.inner.write_buf(buf)
    }
}

impl<T: IoStream> IoStream for Io<T> {
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

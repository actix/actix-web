use std::marker::PhantomData;
use std::{fmt, io};

use actix_codec::{AsyncRead, AsyncWrite, Framed, FramedParts};
use actix_server_config::{Io as ServerIo, Protocol, ServerConfig as SrvConfig};
use actix_service::{IntoNewService, NewService, Service};
use actix_utils::cloneable::CloneableService;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{try_ready, Async, Future, IntoFuture, Poll};
use h2::server::{self, Handshake};

use crate::body::MessageBody;
use crate::builder::HttpServiceBuilder;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::{DispatchError, Error};
use crate::request::Request;
use crate::response::Response;
use crate::{h1, h2::Dispatcher};

/// `NewService` HTTP1.1/HTTP2 transport implementation
pub struct HttpService<T, P, S, B, X = h1::ExpectHandler> {
    srv: S,
    cfg: ServiceConfig,
    expect: X,
    _t: PhantomData<(T, P, B)>,
}

impl<T, S, B> HttpService<T, (), S, B>
where
    S: NewService<SrvConfig, Request = Request>,
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
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S, SrvConfig>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        HttpService {
            cfg,
            srv: service.into_new_service(),
            expect: h1::ExpectHandler,
            _t: PhantomData,
        }
    }

    /// Create new `HttpService` instance with config.
    pub(crate) fn with_config<F: IntoNewService<S, SrvConfig>>(
        cfg: ServiceConfig,
        service: F,
    ) -> Self {
        HttpService {
            cfg,
            srv: service.into_new_service(),
            expect: h1::ExpectHandler,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X> HttpService<T, P, S, B, X>
where
    S: NewService<SrvConfig, Request = Request>,
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
    pub fn expect<U>(self, expect: U) -> HttpService<T, P, S, B, U>
    where
        U: NewService<Request = Request, Response = Request>,
        U::Error: Into<Error>,
        U::InitError: fmt::Debug,
    {
        HttpService {
            expect,
            cfg: self.cfg,
            srv: self.srv,
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X> NewService<SrvConfig> for HttpService<T, P, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
{
    type Request = ServerIo<T, P>;
    type Response = ();
    type Error = DispatchError;
    type InitError = ();
    type Service = HttpServiceHandler<T, P, S::Service, B, X::Service>;
    type Future = HttpServiceResponse<T, P, S, B, X>;

    fn new_service(&self, cfg: &SrvConfig) -> Self::Future {
        HttpServiceResponse {
            fut: self.srv.new_service(cfg).into_future(),
            fut_ex: Some(self.expect.new_service(&())),
            expect: None,
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct HttpServiceResponse<T, P, S: NewService<SrvConfig>, B, X: NewService> {
    fut: S::Future,
    fut_ex: Option<X::Future>,
    expect: Option<X::Service>,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B, X> Future for HttpServiceResponse<T, P, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<SrvConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    <S::Service as Service>::Future: 'static,
    B: MessageBody + 'static,
    X: NewService<Request = Request, Response = Request>,
    X::Error: Into<Error>,
    X::InitError: fmt::Debug,
{
    type Item = HttpServiceHandler<T, P, S::Service, B, X::Service>;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut fut) = self.fut_ex {
            let expect = try_ready!(fut
                .poll()
                .map_err(|e| log::error!("Init http service error: {:?}", e)));
            self.expect = Some(expect);
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
        )))
    }
}

/// `Service` implementation for http transport
pub struct HttpServiceHandler<T, P, S, B, X> {
    srv: CloneableService<S>,
    expect: CloneableService<X>,
    cfg: ServiceConfig,
    _t: PhantomData<(T, P, B, X)>,
}

impl<T, P, S, B, X> HttpServiceHandler<T, P, S, B, X>
where
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
{
    fn new(cfg: ServiceConfig, srv: S, expect: X) -> HttpServiceHandler<T, P, S, B, X> {
        HttpServiceHandler {
            cfg,
            srv: CloneableService::new(srv),
            expect: CloneableService::new(expect),
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B, X> Service for HttpServiceHandler<T, P, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
{
    type Request = ServerIo<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = HttpServiceHandlerResponse<T, S, B, X>;

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
        match proto {
            Protocol::Http2 => {
                let io = Io {
                    inner: io,
                    unread: None,
                };
                HttpServiceHandlerResponse {
                    state: State::Handshake(Some((
                        server::handshake(io),
                        self.cfg.clone(),
                        self.srv.clone(),
                    ))),
                }
            }
            Protocol::Http10 | Protocol::Http11 => HttpServiceHandlerResponse {
                state: State::H1(h1::Dispatcher::new(
                    io,
                    self.cfg.clone(),
                    self.srv.clone(),
                    self.expect.clone(),
                )),
            },
            _ => HttpServiceHandlerResponse {
                state: State::Unknown(Some((
                    io,
                    BytesMut::with_capacity(14),
                    self.cfg.clone(),
                    self.srv.clone(),
                    self.expect.clone(),
                ))),
            },
        }
    }
}

enum State<T, S, B, X>
where
    S: Service<Request = Request>,
    S::Future: 'static,
    S::Error: Into<Error>,
    T: AsyncRead + AsyncWrite,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
{
    H1(h1::Dispatcher<T, S, B, X>),
    H2(Dispatcher<Io<T>, S, B>),
    Unknown(
        Option<(
            T,
            BytesMut,
            ServiceConfig,
            CloneableService<S>,
            CloneableService<X>,
        )>,
    ),
    Handshake(Option<(Handshake<Io<T>, Bytes>, ServiceConfig, CloneableService<S>)>),
}

pub struct HttpServiceHandlerResponse<T, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
{
    state: State<T, S, B, X>,
}

const HTTP2_PREFACE: [u8; 14] = *b"PRI * HTTP/2.0";

impl<T, S, B, X> Future for HttpServiceHandlerResponse<T, S, B, X>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request>,
    S::Error: Into<Error>,
    S::Future: 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody,
    X: Service<Request = Request, Response = Request>,
    X::Error: Into<Error>,
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
                        unsafe {
                            let b = item.1.bytes_mut();
                            let n = try_ready!(item.0.poll_read(b));
                            if n == 0 {
                                return Ok(Async::Ready(()));
                            }
                            item.1.advance_mut(n);
                            if item.1.len() >= HTTP2_PREFACE.len() {
                                break;
                            }
                        }
                    }
                } else {
                    panic!()
                }
                let (io, buf, cfg, srv, expect) = data.take().unwrap();
                if buf[..14] == HTTP2_PREFACE[..] {
                    let io = Io {
                        inner: io,
                        unread: Some(buf),
                    };
                    self.state =
                        State::Handshake(Some((server::handshake(io), cfg, srv)));
                } else {
                    let framed = Framed::from_parts(FramedParts::with_read_buf(
                        io,
                        h1::Codec::new(cfg.clone()),
                        buf,
                    ));
                    self.state = State::H1(h1::Dispatcher::with_timeout(
                        framed, cfg, None, srv, expect,
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
                let (_, cfg, srv) = data.take().unwrap();
                self.state = State::H2(Dispatcher::new(srv, conn, cfg, None));
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

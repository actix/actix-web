use std::fmt::Debug;
use std::marker::PhantomData;
use std::{fmt, io};

use actix_codec::{AsyncRead, AsyncWrite, Framed, FramedParts};
use actix_server_config::{Io as ServerIo, Protocol, ServerConfig as SrvConfig};
use actix_service::{IntoNewService, NewService, Service};
use actix_utils::cloneable::CloneableService;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{try_ready, Async, Future, IntoFuture, Poll};
use h2::server::{self, Handshake};
use log::error;

use crate::body::MessageBody;
use crate::builder::HttpServiceBuilder;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::DispatchError;
use crate::request::Request;
use crate::response::Response;
use crate::{h1, h2::Dispatcher};

/// `NewService` HTTP1.1/HTTP2 transport implementation
pub struct HttpService<T, P, S, B> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<(T, P, B)>,
}

impl<T, S, B> HttpService<T, (), S, B>
where
    S: NewService<SrvConfig, Request = Request>,
    S::Service: 'static,
    S::Error: Debug + 'static,
    S::Response: Into<Response<B>>,
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
    S::Service: 'static,
    S::Error: Debug + 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S, SrvConfig>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        HttpService {
            cfg,
            srv: service.into_new_service(),
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
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B> NewService<SrvConfig> for HttpService<T, P, S, B>
where
    T: AsyncRead + AsyncWrite + 'static,
    S: NewService<SrvConfig, Request = Request>,
    S::Service: 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    type Request = ServerIo<T, P>;
    type Response = ();
    type Error = DispatchError;
    type InitError = S::InitError;
    type Service = HttpServiceHandler<T, P, S::Service, B>;
    type Future = HttpServiceResponse<T, P, S, B>;

    fn new_service(&self, cfg: &SrvConfig) -> Self::Future {
        HttpServiceResponse {
            fut: self.srv.new_service(cfg).into_future(),
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct HttpServiceResponse<T, P, S: NewService<SrvConfig>, B> {
    fut: <S::Future as IntoFuture>::Future,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> Future for HttpServiceResponse<T, P, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<SrvConfig, Request = Request>,
    S::Service: 'static,
    S::Response: Into<Response<B>>,
    S::Error: Debug,
    B: MessageBody + 'static,
{
    type Item = HttpServiceHandler<T, P, S::Service, B>;
    type Error = S::InitError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let service = try_ready!(self.fut.poll());
        Ok(Async::Ready(HttpServiceHandler::new(
            self.cfg.take().unwrap(),
            service,
        )))
    }
}

/// `Service` implementation for http transport
pub struct HttpServiceHandler<T, P, S: 'static, B> {
    srv: CloneableService<S>,
    cfg: ServiceConfig,
    _t: PhantomData<(T, P, B)>,
}

impl<T, P, S, B> HttpServiceHandler<T, P, S, B>
where
    S: Service<Request = Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    fn new(cfg: ServiceConfig, srv: S) -> HttpServiceHandler<T, P, S, B> {
        HttpServiceHandler {
            cfg,
            srv: CloneableService::new(srv),
            _t: PhantomData,
        }
    }
}

impl<T, P, S, B> Service for HttpServiceHandler<T, P, S, B>
where
    T: AsyncRead + AsyncWrite + 'static,
    S: Service<Request = Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    type Request = ServerIo<T, P>;
    type Response = ();
    type Error = DispatchError;
    type Future = HttpServiceHandlerResponse<T, S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.srv.poll_ready().map_err(|e| {
            error!("Service readiness error: {:?}", e);
            DispatchError::Service
        })
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let (io, params, proto) = req.into_parts();
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
                )),
            },
            _ => HttpServiceHandlerResponse {
                state: State::Unknown(Some((
                    io,
                    BytesMut::with_capacity(14),
                    self.cfg.clone(),
                    self.srv.clone(),
                ))),
            },
        }
    }
}

enum State<T, S: Service<Request = Request> + 'static, B: MessageBody>
where
    S::Error: fmt::Debug,
    T: AsyncRead + AsyncWrite + 'static,
{
    H1(h1::Dispatcher<T, S, B>),
    H2(Dispatcher<Io<T>, S, B>),
    Unknown(Option<(T, BytesMut, ServiceConfig, CloneableService<S>)>),
    Handshake(Option<(Handshake<Io<T>, Bytes>, ServiceConfig, CloneableService<S>)>),
}

pub struct HttpServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite + 'static,
    S: Service<Request = Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    state: State<T, S, B>,
}

const HTTP2_PREFACE: [u8; 14] = *b"PRI * HTTP/2.0";

impl<T, S, B> Future for HttpServiceHandlerResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: Service<Request = Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
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
                            let n = { try_ready!(item.0.poll_read(b)) };
                            item.1.advance_mut(n);
                            if item.1.len() >= HTTP2_PREFACE.len() {
                                break;
                            }
                        }
                    }
                } else {
                    panic!()
                }
                let (io, buf, cfg, srv) = data.take().unwrap();
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
                    self.state =
                        State::H1(h1::Dispatcher::with_timeout(framed, cfg, None, srv))
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

impl<T: AsyncRead + 'static> AsyncRead for Io<T> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.inner.prepare_uninitialized_buffer(buf)
    }
}

impl<T: AsyncWrite + 'static> AsyncWrite for Io<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.inner.shutdown()
    }
    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.inner.write_buf(buf)
    }
}

use std::fmt::Debug;
use std::marker::PhantomData;
use std::{fmt, io, net};

use actix_codec::{AsyncRead, AsyncWrite, Framed, FramedParts};
use actix_service::{IntoNewService, NewService, Service};
use actix_utils::cloneable::CloneableService;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{try_ready, Async, Future, IntoFuture, Poll};
use h2::server::{self, Handshake};
use log::error;

use crate::body::MessageBody;
use crate::config::{KeepAlive, ServiceConfig};
use crate::error::DispatchError;
use crate::request::Request;
use crate::response::Response;

use crate::{h1, h2::Dispatcher};

/// `NewService` HTTP1.1/HTTP2 transport implementation
pub struct HttpService<T, S, B> {
    srv: S,
    cfg: ServiceConfig,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> HttpService<T, S, B>
where
    S: NewService<Request>,
    S::Service: 'static,
    S::Error: Debug + 'static,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    /// Create new `HttpService` instance.
    pub fn new<F: IntoNewService<S, Request>>(service: F) -> Self {
        let cfg = ServiceConfig::new(KeepAlive::Timeout(5), 5000, 0);

        HttpService {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }

    /// Create builder for `HttpService` instance.
    pub fn build() -> HttpServiceBuilder<T, S> {
        HttpServiceBuilder::new()
    }
}

impl<T, S, B> NewService<T> for HttpService<T, S, B>
where
    T: AsyncRead + AsyncWrite + 'static,
    S: NewService<Request>,
    S::Service: 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    type Response = ();
    type Error = DispatchError;
    type InitError = S::InitError;
    type Service = HttpServiceHandler<T, S::Service, B>;
    type Future = HttpServiceResponse<T, S, B>;

    fn new_service(&self, _: &()) -> Self::Future {
        HttpServiceResponse {
            fut: self.srv.new_service(&()).into_future(),
            cfg: Some(self.cfg.clone()),
            _t: PhantomData,
        }
    }
}

/// A http service factory builder
///
/// This type can be used to construct an instance of `ServiceConfig` through a
/// builder-like pattern.
pub struct HttpServiceBuilder<T, S> {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_disconnect: u64,
    host: String,
    addr: net::SocketAddr,
    secure: bool,
    _t: PhantomData<(T, S)>,
}

impl<T, S> HttpServiceBuilder<T, S>
where
    S: NewService<Request>,
    S::Service: 'static,
    S::Error: Debug + 'static,
{
    /// Create instance of `HttpServiceBuilder` type
    pub fn new() -> HttpServiceBuilder<T, S> {
        HttpServiceBuilder {
            keep_alive: KeepAlive::Timeout(5),
            client_timeout: 5000,
            client_disconnect: 0,
            secure: false,
            host: "localhost".to_owned(),
            addr: "127.0.0.1:8080".parse().unwrap(),
            _t: PhantomData,
        }
    }

    /// Enable secure flag for current server.
    /// This flags also enables `client disconnect timeout`.
    ///
    /// By default this flag is set to false.
    pub fn secure(mut self) -> Self {
        self.secure = true;
        if self.client_disconnect == 0 {
            self.client_disconnect = 3000;
        }
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<U: Into<KeepAlive>>(mut self, val: U) -> Self {
        self.keep_alive = val.into();
        self
    }

    /// Set server client timeout in milliseconds for first request.
    ///
    /// Defines a timeout for reading client request header. If a client does not transmit
    /// the entire set headers within this time, the request is terminated with
    /// the 408 (Request Time-out) error.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default client timeout is set to 5000 milliseconds.
    pub fn client_timeout(mut self, val: u64) -> Self {
        self.client_timeout = val;
        self
    }

    /// Set server connection disconnect timeout in milliseconds.
    ///
    /// Defines a timeout for disconnect connection. If a disconnect procedure does not complete
    /// within this time, the request get dropped. This timeout affects secure connections.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default disconnect timeout is set to 3000 milliseconds.
    pub fn client_disconnect(mut self, val: u64) -> Self {
        self.client_disconnect = val;
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default host name is set to a "localhost" value.
    pub fn server_hostname(mut self, val: &str) -> Self {
        self.host = val.to_owned();
        self
    }

    /// Set server ip address.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    ///
    /// By default server address is set to a "127.0.0.1:8080"
    pub fn server_address<U: net::ToSocketAddrs>(mut self, addr: U) -> Self {
        match addr.to_socket_addrs() {
            Err(err) => error!("Can not convert to SocketAddr: {}", err),
            Ok(mut addrs) => {
                if let Some(addr) = addrs.next() {
                    self.addr = addr;
                }
            }
        }
        self
    }

    // #[cfg(feature = "ssl")]
    // /// Configure alpn protocols for SslAcceptorBuilder.
    // pub fn configure_openssl(
    //     builder: &mut openssl::ssl::SslAcceptorBuilder,
    // ) -> io::Result<()> {
    //     let protos: &[u8] = b"\x02h2";
    //     builder.set_alpn_select_callback(|_, protos| {
    //         const H2: &[u8] = b"\x02h2";
    //         if protos.windows(3).any(|window| window == H2) {
    //             Ok(b"h2")
    //         } else {
    //             Err(openssl::ssl::AlpnError::NOACK)
    //         }
    //     });
    //     builder.set_alpn_protos(&protos)?;

    //     Ok(())
    // }

    /// Finish service configuration and create `HttpService` instance.
    pub fn finish<F, B>(self, service: F) -> HttpService<T, S, B>
    where
        B: MessageBody,
        F: IntoNewService<S, Request>,
    {
        let cfg = ServiceConfig::new(
            self.keep_alive,
            self.client_timeout,
            self.client_disconnect,
        );
        HttpService {
            cfg,
            srv: service.into_new_service(),
            _t: PhantomData,
        }
    }
}

#[doc(hidden)]
pub struct HttpServiceResponse<T, S: NewService<Request>, B> {
    fut: <S::Future as IntoFuture>::Future,
    cfg: Option<ServiceConfig>,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> Future for HttpServiceResponse<T, S, B>
where
    T: AsyncRead + AsyncWrite,
    S: NewService<Request>,
    S::Service: 'static,
    S::Response: Into<Response<B>>,
    S::Error: Debug,
    B: MessageBody + 'static,
{
    type Item = HttpServiceHandler<T, S::Service, B>;
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
pub struct HttpServiceHandler<T, S: 'static, B> {
    srv: CloneableService<S>,
    cfg: ServiceConfig,
    _t: PhantomData<(T, B)>,
}

impl<T, S, B> HttpServiceHandler<T, S, B>
where
    S: Service<Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    fn new(cfg: ServiceConfig, srv: S) -> HttpServiceHandler<T, S, B> {
        HttpServiceHandler {
            cfg,
            srv: CloneableService::new(srv),
            _t: PhantomData,
        }
    }
}

impl<T, S, B> Service<T> for HttpServiceHandler<T, S, B>
where
    T: AsyncRead + AsyncWrite + 'static,
    S: Service<Request> + 'static,
    S::Error: Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody + 'static,
{
    type Response = ();
    type Error = DispatchError;
    type Future = HttpServiceHandlerResponse<T, S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.srv.poll_ready().map_err(|e| {
            error!("Service readiness error: {:?}", e);
            DispatchError::Service
        })
    }

    fn call(&mut self, req: T) -> Self::Future {
        HttpServiceHandlerResponse {
            state: State::Unknown(Some((
                req,
                BytesMut::with_capacity(14),
                self.cfg.clone(),
                self.srv.clone(),
            ))),
        }
    }
}

enum State<T, S: Service<Request> + 'static, B: MessageBody>
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
    S: Service<Request> + 'static,
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
    S: Service<Request> + 'static,
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

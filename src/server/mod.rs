//! Http server
use std::net::Shutdown;
use std::{io, time};

use bytes::BytesMut;
use futures::{Async, Poll};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tcp::TcpStream;

mod channel;
pub(crate) mod encoding;
pub(crate) mod h1;
pub(crate) mod h1decoder;
mod h1writer;
mod h2;
mod h2writer;
pub(crate) mod helpers;
pub(crate) mod settings;
pub(crate) mod shared;
mod srv;
pub(crate) mod utils;
mod worker;

pub use self::settings::ServerSettings;
pub use self::srv::HttpServer;

#[doc(hidden)]
pub use self::helpers::write_content_length;

use actix::Message;
use body::Binary;
use error::Error;
use header::ContentEncoding;
use httprequest::{HttpInnerMessage, HttpRequest};
use httpresponse::HttpResponse;

/// max buffer size 64k
pub(crate) const MAX_WRITE_BUFFER_SIZE: usize = 65_536;

/// Create new http server with application factory.
///
/// This is shortcut for `server::HttpServer::new()` method.
///
/// ```rust
/// # extern crate actix_web;
/// use actix_web::{actix, server, App, HttpResponse};
///
/// fn main() {
///     let sys = actix::System::new("example");  // <- create Actix system
///
///     server::new(
///         || App::new()
///             .resource("/", |r| r.f(|_| HttpResponse::Ok())))
///         .bind("127.0.0.1:59090").unwrap()
///         .start();
///
/// #       actix::System::current().stop();
///     sys.run();
/// }
/// ```
pub fn new<F, U, H>(factory: F) -> HttpServer<H>
where
    F: Fn() -> U + Sync + Send + 'static,
    U: IntoIterator<Item = H> + 'static,
    H: IntoHttpHandler + 'static,
{
    HttpServer::new(factory)
}

#[derive(Debug, PartialEq, Clone, Copy)]
/// Server keep-alive setting
pub enum KeepAlive {
    /// Keep alive in seconds
    Timeout(usize),
    /// Use `SO_KEEPALIVE` socket option, value in seconds
    Tcp(usize),
    /// Relay on OS to shutdown tcp connection
    Os,
    /// Disabled
    Disabled,
}

impl From<usize> for KeepAlive {
    fn from(keepalive: usize) -> Self {
        KeepAlive::Timeout(keepalive)
    }
}

impl From<Option<usize>> for KeepAlive {
    fn from(keepalive: Option<usize>) -> Self {
        if let Some(keepalive) = keepalive {
            KeepAlive::Timeout(keepalive)
        } else {
            KeepAlive::Disabled
        }
    }
}

/// Pause accepting incoming connections
///
/// If socket contains some pending connection, they might be dropped.
/// All opened connection remains active.
#[derive(Message)]
pub struct PauseServer;

/// Resume accepting incoming connections
#[derive(Message)]
pub struct ResumeServer;

/// Stop incoming connection processing, stop all workers and exit.
///
/// If server starts with `spawn()` method, then spawned thread get terminated.
pub struct StopServer {
    /// Whether to try and shut down gracefully
    pub graceful: bool,
}

impl Message for StopServer {
    type Result = Result<(), ()>;
}

/// Low level http request handler
#[allow(unused_variables)]
pub trait HttpHandler: 'static {
    /// Handle request
    fn handle(&mut self, req: HttpRequest) -> Result<Box<HttpHandlerTask>, HttpRequest>;
}

impl HttpHandler for Box<HttpHandler> {
    fn handle(&mut self, req: HttpRequest) -> Result<Box<HttpHandlerTask>, HttpRequest> {
        self.as_mut().handle(req)
    }
}

#[doc(hidden)]
pub trait HttpHandlerTask {
    /// Poll task, this method is used before or after *io* object is available
    fn poll(&mut self) -> Poll<(), Error> {
        Ok(Async::Ready(()))
    }

    /// Poll task when *io* object is available
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error>;

    /// Connection is disconnected
    fn disconnected(&mut self) {}
}

/// Conversion helper trait
pub trait IntoHttpHandler {
    /// The associated type which is result of conversion.
    type Handler: HttpHandler;

    /// Convert into `HttpHandler` object.
    fn into_handler(self, settings: ServerSettings) -> Self::Handler;
}

impl<T: HttpHandler> IntoHttpHandler for T {
    type Handler = T;

    fn into_handler(self, _: ServerSettings) -> Self::Handler {
        self
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub enum WriterState {
    Done,
    Pause,
}

#[doc(hidden)]
/// Stream writer
pub trait Writer {
    /// number of bytes written to the stream
    fn written(&self) -> u64;

    #[doc(hidden)]
    fn set_date(&self, st: &mut BytesMut);

    #[doc(hidden)]
    #[cfg_attr(feature = "cargo-clippy", allow(mut_from_ref))]
    fn buffer(&self) -> &mut BytesMut;

    fn start(
        &mut self, req: &mut HttpInnerMessage, resp: &mut HttpResponse,
        encoding: ContentEncoding,
    ) -> io::Result<WriterState>;

    fn write(&mut self, payload: Binary) -> io::Result<WriterState>;

    fn write_eof(&mut self) -> io::Result<WriterState>;

    fn poll_completed(&mut self, shutdown: bool) -> Poll<(), io::Error>;
}

#[doc(hidden)]
/// Low-level io stream operations
pub trait IoStream: AsyncRead + AsyncWrite + 'static {
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()>;

    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()>;

    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()>;
}

impl IoStream for TcpStream {
    #[inline]
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        TcpStream::shutdown(self, how)
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        TcpStream::set_nodelay(self, nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        TcpStream::set_linger(self, dur)
    }
}

#[cfg(feature = "alpn")]
use tokio_openssl::SslStream;

#[cfg(feature = "alpn")]
impl IoStream for SslStream<TcpStream> {
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

#[cfg(feature = "tls")]
use tokio_tls::TlsStream;

#[cfg(feature = "tls")]
impl IoStream for TlsStream<TcpStream> {
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

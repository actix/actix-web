//! Http server
use std::net::Shutdown;
use std::{io, time};

use bytes::{BufMut, BytesMut};
use futures::{Async, Poll};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tcp::TcpStream;

pub(crate) mod accept;
mod channel;
mod error;
pub(crate) mod h1;
pub(crate) mod h1decoder;
mod h1writer;
mod h2;
mod h2writer;
pub(crate) mod helpers;
pub(crate) mod input;
pub(crate) mod message;
pub(crate) mod output;
pub(crate) mod settings;
mod srv;
mod worker;

pub use self::message::Request;
pub use self::settings::ServerSettings;
pub use self::srv::HttpServer;

#[doc(hidden)]
pub use self::helpers::write_content_length;

use actix::Message;
use body::Binary;
use error::Error;
use header::ContentEncoding;
use httpresponse::HttpResponse;

/// max buffer size 64k
pub(crate) const MAX_WRITE_BUFFER_SIZE: usize = 65_536;

const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;

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
    /// Request handling task
    type Task: HttpHandlerTask;

    /// Handle request
    fn handle(&self, req: Request) -> Result<Self::Task, Request>;
}

impl HttpHandler for Box<HttpHandler<Task = Box<HttpHandlerTask>>> {
    type Task = Box<HttpHandlerTask>;

    fn handle(&self, req: Request) -> Result<Box<HttpHandlerTask>, Request> {
        self.as_ref().handle(req)
    }
}

/// Low level http request handler
pub trait HttpHandlerTask {
    /// Poll task, this method is used before or after *io* object is available
    fn poll_completed(&mut self) -> Poll<(), Error> {
        Ok(Async::Ready(()))
    }

    /// Poll task when *io* object is available
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error>;

    /// Connection is disconnected
    fn disconnected(&mut self) {}
}

impl HttpHandlerTask for Box<HttpHandlerTask> {
    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error> {
        self.as_mut().poll_io(io)
    }
}

/// Conversion helper trait
pub trait IntoHttpHandler {
    /// The associated type which is result of conversion.
    type Handler: HttpHandler;

    /// Convert into `HttpHandler` object.
    fn into_handler(self) -> Self::Handler;
}

impl<T: HttpHandler> IntoHttpHandler for T {
    type Handler = T;

    fn into_handler(self) -> Self::Handler {
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
    fn set_date(&mut self);

    #[doc(hidden)]
    fn buffer(&mut self) -> &mut BytesMut;

    fn start(
        &mut self, req: &Request, resp: &mut HttpResponse, encoding: ContentEncoding,
    ) -> io::Result<WriterState>;

    fn write(&mut self, payload: &Binary) -> io::Result<WriterState>;

    fn write_eof(&mut self) -> io::Result<WriterState>;

    fn poll_completed(&mut self, shutdown: bool) -> Poll<(), io::Error>;
}

#[doc(hidden)]
/// Low-level io stream operations
pub trait IoStream: AsyncRead + AsyncWrite + 'static {
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()>;

    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()>;

    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()>;

    fn read_available(&mut self, buf: &mut BytesMut) -> Poll<bool, io::Error> {
        let mut read_some = false;
        loop {
            if buf.remaining_mut() < LW_BUFFER_SIZE {
                buf.reserve(HW_BUFFER_SIZE);
            }
            unsafe {
                match self.read(buf.bytes_mut()) {
                    Ok(n) => {
                        if n == 0 {
                            return Ok(Async::Ready(!read_some));
                        } else {
                            read_some = true;
                            buf.advance_mut(n);
                        }
                    }
                    Err(e) => {
                        return if e.kind() == io::ErrorKind::WouldBlock {
                            if read_some {
                                Ok(Async::Ready(false))
                            } else {
                                Ok(Async::NotReady)
                            }
                        } else {
                            Err(e)
                        };
                    }
                }
            }
        }
    }
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

#[cfg(feature = "rust-tls")]
use rustls::{ClientSession, ServerSession};
#[cfg(feature = "rust-tls")]
use tokio_rustls::TlsStream;

#[cfg(feature = "rust-tls")]
impl IoStream for TlsStream<TcpStream, ClientSession> {
    #[inline]
    fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
        let _ = <Self as AsyncWrite>::shutdown(self);
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.get_mut().0.set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().0.set_linger(dur)
    }
}

#[cfg(feature = "rust-tls")]
impl IoStream for TlsStream<TcpStream, ServerSession> {
    #[inline]
    fn shutdown(&mut self, _how: Shutdown) -> io::Result<()> {
        let _ = <Self as AsyncWrite>::shutdown(self);
        Ok(())
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.get_mut().0.set_nodelay(nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        self.get_mut().0.set_linger(dur)
    }
}

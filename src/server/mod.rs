//! Http server module
//!
//! The module contains everything necessary to setup
//! HTTP server.
//!
//! In order to start HTTP server, first you need to create and configure it
//! using factory that can be supplied to [new](fn.new.html).
//!
//! ## Factory
//!
//! Factory is a function that returns Application, describing how
//! to serve incoming HTTP requests.
//!
//! As the server uses worker pool, the factory function is restricted to trait bounds
//! `Sync + Send + 'static` so that each worker would be able to accept Application
//! without a need for synchronization.
//!
//! If you wish to share part of state among all workers you should
//! wrap it in `Arc` and potentially synchronization primitive like
//! [RwLock](https://doc.rust-lang.org/std/sync/struct.RwLock.html)
//! If the wrapped type is not thread safe.
//!
//! Note though that locking is not advisable for asynchronous programming
//! and you should minimize all locks in your request handlers
//!
//! ## HTTPS Support
//!
//! Actix-web provides support for major crates that provides TLS.
//! Each TLS implementation is provided with [AcceptorService](trait.AcceptorService.html)
//! that describes how HTTP Server accepts connections.
//!
//! For `bind` and `listen` there are corresponding `bind_with` and `listen_with` that accepts
//! these services.
//!
//! By default, acceptor would work with both HTTP2 and HTTP1 protocols.
//! But it can be controlled using [ServerFlags](struct.ServerFlags.html) which
//! can be supplied when creating `AcceptorService`.
//!
//! **NOTE:** `native-tls` doesn't support `HTTP2` yet
//!
//! ## Signal handling and shutdown
//!
//! By default HTTP Server listens for system signals
//! and, gracefully shuts down at most after 30 seconds.
//!
//! Both signal handling and shutdown timeout can be controlled
//! using corresponding methods.
//!
//! If worker, for some reason, unable to shut down within timeout
//! it is forcibly dropped.
//!
//! ## Example
//!
//! ```rust,ignore
//!extern crate actix;
//!extern crate actix_web;
//!extern crate rustls;
//!
//!use actix_web::{http, middleware, server, App, Error, HttpRequest, HttpResponse, Responder};
//!use std::io::BufReader;
//!use rustls::internal::pemfile::{certs, rsa_private_keys};
//!use rustls::{NoClientAuth, ServerConfig};
//!
//!fn index(req: &HttpRequest) -> Result<HttpResponse, Error> {
//!    Ok(HttpResponse::Ok().content_type("text/plain").body("Welcome!"))
//!}
//!
//!fn load_ssl() -> ServerConfig {
//!    use std::io::BufReader;
//!
//!    const CERT: &'static [u8] = include_bytes!("../cert.pem");
//!    const KEY: &'static [u8] = include_bytes!("../key.pem");
//!
//!    let mut cert = BufReader::new(CERT);
//!    let mut key = BufReader::new(KEY);
//!
//!    let mut config = ServerConfig::new(NoClientAuth::new());
//!    let cert_chain = certs(&mut cert).unwrap();
//!    let mut keys = rsa_private_keys(&mut key).unwrap();
//!    config.set_single_cert(cert_chain, keys.remove(0)).unwrap();
//!
//!    config
//!}
//!
//!fn main() {
//!    let sys = actix::System::new("http-server");
//!    // load ssl keys
//!    let config = load_ssl();
//!
//!     // Create acceptor service for only HTTP1 protocol
//!     // You can use ::new(config) to leave defaults
//!     let acceptor = server::RustlsAcceptor::with_flags(config, actix_web::server::ServerFlags::HTTP1);
//!
//!     // create and start server at once
//!     server::new(|| {
//!         App::new()
//!             // register simple handler, handle all methods
//!             .resource("/index.html", |r| r.f(index))
//!             }))
//!     }).bind_with("127.0.0.1:8080", acceptor)
//!     .unwrap()
//!     .start();
//!
//!     println!("Started http server: 127.0.0.1:8080");
//!     //Run system so that server would start accepting connections
//!     let _ = sys.run();
//!}
//! ```
use std::net::Shutdown;
use std::rc::Rc;
use std::{io, net, time};

use bytes::{BufMut, BytesMut};
use futures::{Async, Future, Poll};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_reactor::Handle;
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
mod server;
pub(crate) mod settings;
mod http;
mod ssl;
mod worker;

use actix::Message;

pub use self::message::Request;

#[doc(hidden)]
pub use self::server::{
    ConnectionRateTag, ConnectionTag, Connections, Server, Service, ServiceHandler,
};
pub use self::settings::ServerSettings;
pub use self::http::HttpServer;

#[doc(hidden)]
pub use self::ssl::*;

#[doc(hidden)]
pub use self::helpers::write_content_length;

use body::Binary;
use error::Error;
use extensions::Extensions;
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

bitflags! {
    ///Flags that can be used to configure HTTP Server.
    pub struct ServerFlags: u8 {
        ///Use HTTP1 protocol
        const HTTP1 = 0b0000_0001;
        ///Use HTTP2 protocol
        const HTTP2 = 0b0000_0010;
    }
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

/// Socket id token
#[derive(Clone, Copy)]
pub struct Token(usize);

impl Token {
    pub(crate) fn new(val: usize) -> Token {
        Token(val)
    }
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

pub(crate) trait IntoAsyncIo {
    type Io: AsyncRead + AsyncWrite;

    fn into_async_io(self) -> Result<Self::Io, io::Error>;
}

impl IntoAsyncIo for net::TcpStream {
    type Io = TcpStream;

    fn into_async_io(self) -> Result<Self::Io, io::Error> {
        TcpStream::from_std(self, &Handle::default())
    }
}

/// Trait implemented by types that could accept incomming socket connections.
pub trait AcceptorService<Io: AsyncRead + AsyncWrite>: Clone {
    /// Established connection type
    type Accepted: IoStream;
    /// Future describes async accept process.
    type Future: Future<Item = Self::Accepted, Error = io::Error> + 'static;

    /// Establish new connection
    fn accept(&self, io: Io) -> Self::Future;

    /// Scheme
    fn scheme(&self) -> &'static str;
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

    /// Extra io stream extensions
    fn extensions(&self) -> Option<Rc<Extensions>> {
        None
    }
}

#[cfg(all(unix, feature = "uds"))]
impl IoStream for ::tokio_uds::UnixStream {
    #[inline]
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        ::tokio_uds::UnixStream::shutdown(self, how)
    }

    #[inline]
    fn set_nodelay(&mut self, _nodelay: bool) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_linger(&mut self, _dur: Option<time::Duration>) -> io::Result<()> {
        Ok(())
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

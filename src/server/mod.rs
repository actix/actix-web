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
use std::net::SocketAddr;
use std::{io, time};

use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tcp::TcpStream;

pub use actix_net::server::{PauseServer, ResumeServer, StopServer};

#[doc(hidden)]
pub use super::helpers::write_content_length;

// /// max buffer size 64k
// pub(crate) const MAX_WRITE_BUFFER_SIZE: usize = 65_536;

#[derive(Debug, PartialEq, Clone, Copy)]
/// Server keep-alive setting
pub enum KeepAlive {
    /// Keep alive in seconds
    Timeout(usize),
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

#[doc(hidden)]
/// Low-level io stream operations
pub trait IoStream: AsyncRead + AsyncWrite + 'static {
    /// Returns the socket address of the remote peer of this TCP connection.
    fn peer_addr(&self) -> Option<SocketAddr> {
        None
    }

    /// Sets the value of the TCP_NODELAY option on this socket.
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()>;

    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()>;

    fn set_keepalive(&mut self, dur: Option<time::Duration>) -> io::Result<()>;
}

#[cfg(all(unix, feature = "uds"))]
impl IoStream for ::tokio_uds::UnixStream {
    #[inline]
    fn set_nodelay(&mut self, _nodelay: bool) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_linger(&mut self, _dur: Option<time::Duration>) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    fn set_keepalive(&mut self, _nodelay: bool) -> io::Result<()> {
        Ok(())
    }
}

impl IoStream for TcpStream {
    #[inline]
    fn peer_addr(&self) -> Option<SocketAddr> {
        TcpStream::peer_addr(self).ok()
    }

    #[inline]
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        TcpStream::set_nodelay(self, nodelay)
    }

    #[inline]
    fn set_linger(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        TcpStream::set_linger(self, dur)
    }

    #[inline]
    fn set_keepalive(&mut self, dur: Option<time::Duration>) -> io::Result<()> {
        TcpStream::set_keepalive(self, dur)
    }
}

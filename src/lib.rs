//! Actix web is a small, pragmatic, and extremely fast web framework
//! for Rust.
//!
//! ## Package feature
//!
//! * `tls` - enables ssl support via `native-tls` crate
//! * `alpn` - enables ssl support via `openssl` crate, require for `http/2`
//!    support
//! * `rust-tls` - enables ssl support via `rustls` crate
//!

// #![warn(missing_docs)]
// #![allow(
//     dead_code,
//     unused_variables,
//     unused_imports,
//     patterns_in_fns_without_body
// )]

#[macro_use]
extern crate log;
extern crate byteorder;
extern crate bytes;
extern crate failure;
extern crate futures;
extern crate mio;
extern crate net2;
extern crate num_cpus;
extern crate parking_lot;
extern crate slab;
extern crate time;
extern crate tokio;
extern crate tokio_io;
extern crate tokio_reactor;
extern crate tokio_tcp;
extern crate tokio_timer;
extern crate tower_service;

#[macro_use]
extern crate actix;

#[cfg(feature = "tls")]
extern crate native_tls;

#[cfg(feature = "ssl")]
extern crate openssl;
#[cfg(feature = "ssl")]
extern crate tokio_openssl;

#[cfg(feature = "rust-tls")]
extern crate rustls;
#[cfg(feature = "rust-tls")]
extern crate tokio_rustls;
#[cfg(feature = "rust-tls")]
extern crate webpki;
#[cfg(feature = "rust-tls")]
extern crate webpki_roots;

use std::io;
use std::net::Shutdown;
use std::rc::Rc;

use actix::Message;
use bytes::{BufMut, BytesMut};
use futures::{Async, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

pub(crate) mod accept;
mod extensions;
mod server;
mod server_service;
pub mod service;
pub mod ssl;
mod worker;

pub use self::server::{ConnectionRateTag, ConnectionTag, Connections, Server};
pub use service::{IntoNewService, IntoService};

pub use extensions::Extensions;

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

// impl Token {
//     pub(crate) fn new(val: usize) -> Token {
//         Token(val)
//     }
// }

const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;

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

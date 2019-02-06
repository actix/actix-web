use std::{fmt, io, mem, net};

use actix::{Addr, System};
use actix_net::server::Server;
use actix_net::service::NewService;
use actix_net::ssl;

use net2::TcpBuilder;
use num_cpus;

#[cfg(feature = "tls")]
use native_tls::TlsAcceptor;

#[cfg(any(feature = "alpn", feature = "ssl"))]
use openssl::ssl::SslAcceptorBuilder;

#[cfg(feature = "rust-tls")]
use rustls::ServerConfig;

use super::acceptor::{AcceptorServiceFactory, DefaultAcceptor};
use super::builder::{HttpServiceBuilder, ServiceProvider};
use super::{IntoHttpHandler, KeepAlive};

struct Socket {
    scheme: &'static str,
    lst: net::TcpListener,
    addr: net::SocketAddr,
    handler: Box<ServiceProvider>,
}

/// An HTTP Server
///
/// By default it serves HTTP2 when HTTPs is enabled,
/// in order to change it, use `ServerFlags` that can be provided
/// to acceptor service.
pub struct HttpServer<H, F>
where
    H: IntoHttpHandler + 'static,
    F: Fn() -> H + Send + Clone,
{
    pub(super) factory: F,
    pub(super) host: Option<String>,
    pub(super) keep_alive: KeepAlive,
    pub(super) client_timeout: u64,
    pub(super) client_shutdown: u64,
    backlog: i32,
    threads: usize,
    exit: bool,
    shutdown_timeout: u16,
    no_http2: bool,
    no_signals: bool,
    maxconn: usize,
    maxconnrate: usize,
    sockets: Vec<Socket>,
}

impl<H, F> HttpServer<H, F>
where
    H: IntoHttpHandler + 'static,
    F: Fn() -> H + Send + Clone + 'static,
{
    /// Create new http server with application factory
    pub fn new(factory: F) -> HttpServer<H, F> {
        HttpServer {
            factory,
            threads: num_cpus::get(),
            host: None,
            backlog: 2048,
            keep_alive: KeepAlive::Timeout(5),
            shutdown_timeout: 30,
            exit: false,
            no_http2: false,
            no_signals: false,
            maxconn: 25_600,
            maxconnrate: 256,
            client_timeout: 5000,
            client_shutdown: 5000,
            sockets: Vec::new(),
        }
    }

    /// Set number of workers to start.
    ///
    /// By default http server uses number of available logical cpu as threads
    /// count.
    pub fn workers(mut self, num: usize) -> Self {
        self.threads = num;
        self
    }

    /// Set the maximum number of pending connections.
    ///
    /// This refers to the number of clients that can be waiting to be served.
    /// Exceeding this number results in the client getting an error when
    /// attempting to connect. It should only affect servers under significant
    /// load.
    ///
    /// Generally set in the 64-2048 range. Default value is 2048.
    ///
    /// This method should be called before `bind()` method call.
    pub fn backlog(mut self, num: i32) -> Self {
        self.backlog = num;
        self
    }

    /// Sets the maximum per-worker number of concurrent connections.
    ///
    /// All socket listeners will stop accepting connections when this limit is reached
    /// for each worker.
    ///
    /// By default max connections is set to a 25k.
    pub fn maxconn(mut self, num: usize) -> Self {
        self.maxconn = num;
        self
    }

    /// Sets the maximum per-worker concurrent connection establish process.
    ///
    /// All listeners will stop accepting connections when this limit is reached. It
    /// can be used to limit the global SSL CPU usage.
    ///
    /// By default max connections is set to a 256.
    pub fn maxconnrate(mut self, num: usize) -> Self {
        self.maxconnrate = num;
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<T: Into<KeepAlive>>(mut self, val: T) -> Self {
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

    /// Set server connection shutdown timeout in milliseconds.
    ///
    /// Defines a timeout for shutdown connection. If a shutdown procedure does not complete
    /// within this time, the request is dropped.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default client timeout is set to 5000 milliseconds.
    pub fn client_shutdown(mut self, val: u64) -> Self {
        self.client_shutdown = val;
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    pub fn server_hostname(mut self, val: String) -> Self {
        self.host = Some(val);
        self
    }

    /// Stop actix system.
    ///
    /// `SystemExit` message stops currently running system.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    /// Disable signal handling
    pub fn disable_signals(mut self) -> Self {
        self.no_signals = true;
        self
    }

    /// Timeout for graceful workers shutdown.
    ///
    /// After receiving a stop signal, workers have this much time to finish
    /// serving requests. Workers still alive after the timeout are force
    /// dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, sec: u16) -> Self {
        self.shutdown_timeout = sec;
        self
    }

    /// Disable `HTTP/2` support
    pub fn no_http2(mut self) -> Self {
        self.no_http2 = true;
        self
    }

    /// Get addresses of bound sockets.
    pub fn addrs(&self) -> Vec<net::SocketAddr> {
        self.sockets.iter().map(|s| s.addr).collect()
    }

    /// Get addresses of bound sockets and the scheme for it.
    ///
    /// This is useful when the server is bound from different sources
    /// with some sockets listening on http and some listening on https
    /// and the user should be presented with an enumeration of which
    /// socket requires which protocol.
    pub fn addrs_with_scheme(&self) -> Vec<(net::SocketAddr, &str)> {
        self.sockets.iter().map(|s| (s.addr, s.scheme)).collect()
    }

    /// Use listener for accepting incoming connection requests
    ///
    /// HttpServer does not change any configuration for TcpListener,
    /// it needs to be configured before passing it to listen() method.
    pub fn listen(mut self, lst: net::TcpListener) -> Self {
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            lst,
            addr,
            scheme: "http",
            handler: Box::new(HttpServiceBuilder::new(
                self.factory.clone(),
                DefaultAcceptor,
            )),
        });

        self
    }

    #[doc(hidden)]
    /// Use listener for accepting incoming connection requests
    pub fn listen_with<A>(mut self, lst: net::TcpListener, acceptor: A) -> Self
    where
        A: AcceptorServiceFactory,
        <A::NewService as NewService>::InitError: fmt::Debug,
    {
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            lst,
            addr,
            scheme: "https",
            handler: Box::new(HttpServiceBuilder::new(self.factory.clone(), acceptor)),
        });

        self
    }

    #[cfg(feature = "tls")]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// HttpServer does not change any configuration for TcpListener,
    /// it needs to be configured before passing it to listen() method.
    pub fn listen_tls(self, lst: net::TcpListener, acceptor: TlsAcceptor) -> Self {
        use actix_net::service::NewServiceExt;

        self.listen_with(lst, move || {
            ssl::NativeTlsAcceptor::new(acceptor.clone()).map_err(|_| ())
        })
    }

    #[cfg(any(feature = "alpn", feature = "ssl"))]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_ssl(
        self, lst: net::TcpListener, builder: SslAcceptorBuilder,
    ) -> io::Result<Self> {
        use super::{openssl_acceptor_with_flags, ServerFlags};
        use actix_net::service::NewServiceExt;

        let flags = if self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        let acceptor = openssl_acceptor_with_flags(builder, flags)?;
        Ok(self.listen_with(lst, move || {
            ssl::OpensslAcceptor::new(acceptor.clone()).map_err(|_| ())
        }))
    }

    #[cfg(feature = "rust-tls")]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_rustls(self, lst: net::TcpListener, config: ServerConfig) -> Self {
        use super::{RustlsAcceptor, ServerFlags};
        use actix_net::service::NewServiceExt;

        // alpn support
        let flags = if self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        self.listen_with(lst, move || {
            RustlsAcceptor::with_flags(config.clone(), flags).map_err(|_| ())
        })
    }

    /// The socket address to bind
    ///
    /// To bind multiple addresses this method can be called multiple times.
    pub fn bind<S: net::ToSocketAddrs>(mut self, addr: S) -> io::Result<Self> {
        let sockets = self.bind2(addr)?;

        for lst in sockets {
            self = self.listen(lst);
        }

        Ok(self)
    }

    /// Start listening for incoming connections with supplied acceptor.
    #[doc(hidden)]
    #[cfg_attr(
        feature = "cargo-clippy",
        allow(needless_pass_by_value)
    )]
    pub fn bind_with<S, A>(mut self, addr: S, acceptor: A) -> io::Result<Self>
    where
        S: net::ToSocketAddrs,
        A: AcceptorServiceFactory,
        <A::NewService as NewService>::InitError: fmt::Debug,
    {
        let sockets = self.bind2(addr)?;

        for lst in sockets {
            let addr = lst.local_addr().unwrap();
            self.sockets.push(Socket {
                lst,
                addr,
                scheme: "https",
                handler: Box::new(HttpServiceBuilder::new(
                    self.factory.clone(),
                    acceptor.clone(),
                )),
            });
        }

        Ok(self)
    }

    fn bind2<S: net::ToSocketAddrs>(
        &self, addr: S,
    ) -> io::Result<Vec<net::TcpListener>> {
        let mut err = None;
        let mut succ = false;
        let mut sockets = Vec::new();
        for addr in addr.to_socket_addrs()? {
            match create_tcp_listener(addr, self.backlog) {
                Ok(lst) => {
                    succ = true;
                    sockets.push(lst);
                }
                Err(e) => err = Some(e),
            }
        }

        if !succ {
            if let Some(e) = err.take() {
                Err(e)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Can not bind to address.",
                ))
            }
        } else {
            Ok(sockets)
        }
    }

    #[cfg(feature = "tls")]
    /// The ssl socket address to bind
    ///
    /// To bind multiple addresses this method can be called multiple times.
    pub fn bind_tls<S: net::ToSocketAddrs>(
        self, addr: S, acceptor: TlsAcceptor,
    ) -> io::Result<Self> {
        use actix_net::service::NewServiceExt;
        use actix_net::ssl::NativeTlsAcceptor;

        self.bind_with(addr, move || {
            NativeTlsAcceptor::new(acceptor.clone()).map_err(|_| ())
        })
    }

    #[cfg(any(feature = "alpn", feature = "ssl"))]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_ssl<S>(self, addr: S, builder: SslAcceptorBuilder) -> io::Result<Self>
    where
        S: net::ToSocketAddrs,
    {
        use super::{openssl_acceptor_with_flags, ServerFlags};
        use actix_net::service::NewServiceExt;

        // alpn support
        let flags = if self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        let acceptor = openssl_acceptor_with_flags(builder, flags)?;
        self.bind_with(addr, move || {
            ssl::OpensslAcceptor::new(acceptor.clone()).map_err(|_| ())
        })
    }

    #[cfg(feature = "rust-tls")]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_rustls<S: net::ToSocketAddrs>(
        self, addr: S, builder: ServerConfig,
    ) -> io::Result<Self> {
        use super::{RustlsAcceptor, ServerFlags};
        use actix_net::service::NewServiceExt;

        // alpn support
        let flags = if self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        self.bind_with(addr, move || {
            RustlsAcceptor::with_flags(builder.clone(), flags).map_err(|_| ())
        })
    }
}

impl<H: IntoHttpHandler, F: Fn() -> H + Send + Clone> HttpServer<H, F> {
    /// Start listening for incoming connections.
    ///
    /// This method starts number of http workers in separate threads.
    /// For each address this method starts separate thread which does
    /// `accept()` in a loop.
    ///
    /// This methods panics if no socket address can be bound or an `Actix` system is not yet
    /// configured.
    ///
    /// ```rust
    /// extern crate actix_web;
    /// extern crate actix;
    /// use actix_web::{server, App, HttpResponse};
    ///
    /// fn main() {
    ///     let sys = actix::System::new("example");  // <- create Actix system
    ///
    ///     server::new(|| App::new().resource("/", |r| r.h(|_: &_| HttpResponse::Ok())))
    ///         .bind("127.0.0.1:0")
    ///         .expect("Can not bind to 127.0.0.1:0")
    ///         .start();
    /// #   actix::System::current().stop();
    ///    sys.run();  // <- Run actix system, this method starts all async processes
    /// }
    /// ```
    pub fn start(mut self) -> Addr<Server> {
        ssl::max_concurrent_ssl_connect(self.maxconnrate);

        let mut srv = Server::new()
            .workers(self.threads)
            .maxconn(self.maxconn)
            .shutdown_timeout(self.shutdown_timeout);

        srv = if self.exit { srv.system_exit() } else { srv };
        srv = if self.no_signals {
            srv.disable_signals()
        } else {
            srv
        };

        let sockets = mem::replace(&mut self.sockets, Vec::new());

        for socket in sockets {
            let host = self
                .host
                .as_ref()
                .map(|h| h.to_owned())
                .unwrap_or_else(|| format!("{}", socket.addr));
            let (secure, client_shutdown) = if socket.scheme == "https" {
                (true, self.client_shutdown)
            } else {
                (false, 0)
            };
            srv = socket.handler.register(
                srv,
                socket.lst,
                host,
                socket.addr,
                self.keep_alive,
                secure,
                self.client_timeout,
                client_shutdown,
            );
        }
        srv.start()
    }

    /// Spawn new thread and start listening for incoming connections.
    ///
    /// This method spawns new thread and starts new actix system. Other than
    /// that it is similar to `start()` method. This method blocks.
    ///
    /// This methods panics if no socket addresses get bound.
    ///
    /// ```rust,ignore
    /// # extern crate futures;
    /// # extern crate actix_web;
    /// # use futures::Future;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     HttpServer::new(|| App::new().resource("/", |r| r.h(|_| HttpResponse::Ok())))
    ///         .bind("127.0.0.1:0")
    ///         .expect("Can not bind to 127.0.0.1:0")
    ///         .run();
    /// }
    /// ```
    pub fn run(self) {
        let sys = System::new("http-server");
        self.start();
        sys.run();
    }

    /// Register current http server as actix-net's server service
    pub fn register(self, mut srv: Server) -> Server {
        for socket in self.sockets {
            let host = self
                .host
                .as_ref()
                .map(|h| h.to_owned())
                .unwrap_or_else(|| format!("{}", socket.addr));
            let (secure, client_shutdown) = if socket.scheme == "https" {
                (true, self.client_shutdown)
            } else {
                (false, 0)
            };
            srv = socket.handler.register(
                srv,
                socket.lst,
                host,
                socket.addr,
                self.keep_alive,
                secure,
                self.client_timeout,
                client_shutdown,
            );
        }
        srv
    }
}

fn create_tcp_listener(
    addr: net::SocketAddr, backlog: i32,
) -> io::Result<net::TcpListener> {
    let builder = match addr {
        net::SocketAddr::V4(_) => TcpBuilder::new_v4()?,
        net::SocketAddr::V6(_) => TcpBuilder::new_v6()?,
    };
    builder.reuse_address(true)?;
    builder.bind(addr)?;
    Ok(builder.listen(backlog)?)
}

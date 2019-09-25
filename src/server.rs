use std::marker::PhantomData;
use std::sync::Arc;
use std::{fmt, io, net};

use actix_http::{body::MessageBody, Error, HttpService, KeepAlive, Request, Response};
use actix_rt::System;
use actix_server::{Server, ServerBuilder};
use actix_server_config::ServerConfig;
use actix_service::{IntoNewService, NewService};
use parking_lot::Mutex;

use net2::TcpBuilder;

#[cfg(feature = "ssl")]
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder};
#[cfg(feature = "rust-tls")]
use rustls::ServerConfig as RustlsServerConfig;

struct Socket {
    scheme: &'static str,
    addr: net::SocketAddr,
}

struct Config {
    keep_alive: KeepAlive,
    client_timeout: u64,
    client_shutdown: u64,
}

/// An HTTP Server.
///
/// Create new http server with application factory.
///
/// ```rust
/// use std::io;
/// use actix_web::{web, App, HttpResponse, HttpServer};
///
/// fn main() -> io::Result<()> {
///     let sys = actix_rt::System::new("example");  // <- create Actix runtime
///
///     HttpServer::new(
///         || App::new()
///             .service(web::resource("/").to(|| HttpResponse::Ok())))
///         .bind("127.0.0.1:59090")?
///         .start();
///
/// #       actix_rt::System::current().stop();
///     sys.run()
/// }
/// ```
pub struct HttpServer<F, I, S, B>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoNewService<S>,
    S: NewService<Config = ServerConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    S::Service: 'static,
    B: MessageBody,
{
    pub(super) factory: F,
    pub(super) host: Option<String>,
    config: Arc<Mutex<Config>>,
    backlog: i32,
    sockets: Vec<Socket>,
    builder: ServerBuilder,
    _t: PhantomData<(S, B)>,
}

impl<F, I, S, B> HttpServer<F, I, S, B>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoNewService<S>,
    S: NewService<Config = ServerConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    S::Service: 'static,
    B: MessageBody + 'static,
{
    /// Create new http server with application factory
    pub fn new(factory: F) -> Self {
        HttpServer {
            factory,
            host: None,
            config: Arc::new(Mutex::new(Config {
                keep_alive: KeepAlive::Timeout(5),
                client_timeout: 5000,
                client_shutdown: 5000,
            })),
            backlog: 1024,
            sockets: Vec::new(),
            builder: ServerBuilder::default(),
            _t: PhantomData,
        }
    }

    /// Set number of workers to start.
    ///
    /// By default http server uses number of available logical cpu as threads
    /// count.
    pub fn workers(mut self, num: usize) -> Self {
        self.builder = self.builder.workers(num);
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
    pub fn backlog(mut self, backlog: i32) -> Self {
        self.backlog = backlog;
        self.builder = self.builder.backlog(backlog);
        self
    }

    /// Sets the maximum per-worker number of concurrent connections.
    ///
    /// All socket listeners will stop accepting connections when this limit is reached
    /// for each worker.
    ///
    /// By default max connections is set to a 25k.
    pub fn maxconn(mut self, num: usize) -> Self {
        self.builder = self.builder.maxconn(num);
        self
    }

    /// Sets the maximum per-worker concurrent connection establish process.
    ///
    /// All listeners will stop accepting connections when this limit is reached. It
    /// can be used to limit the global SSL CPU usage.
    ///
    /// By default max connections is set to a 256.
    pub fn maxconnrate(mut self, num: usize) -> Self {
        self.builder = self.builder.maxconnrate(num);
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a 5 seconds.
    pub fn keep_alive<T: Into<KeepAlive>>(self, val: T) -> Self {
        self.config.lock().keep_alive = val.into();
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
    pub fn client_timeout(self, val: u64) -> Self {
        self.config.lock().client_timeout = val;
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
    pub fn client_shutdown(self, val: u64) -> Self {
        self.config.lock().client_shutdown = val;
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router as a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    pub fn server_hostname<T: AsRef<str>>(mut self, val: T) -> Self {
        self.host = Some(val.as_ref().to_owned());
        self
    }

    /// Stop actix system.
    pub fn system_exit(mut self) -> Self {
        self.builder = self.builder.system_exit();
        self
    }

    /// Disable signal handling
    pub fn disable_signals(mut self) -> Self {
        self.builder = self.builder.disable_signals();
        self
    }

    /// Timeout for graceful workers shutdown.
    ///
    /// After receiving a stop signal, workers have this much time to finish
    /// serving requests. Workers still alive after the timeout are force
    /// dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, sec: u64) -> Self {
        self.builder = self.builder.shutdown_timeout(sec);
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
    pub fn listen(mut self, lst: net::TcpListener) -> io::Result<Self> {
        let cfg = self.config.clone();
        let factory = self.factory.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "http",
        });

        self.builder = self.builder.listen(
            format!("actix-web-service-{}", addr),
            lst,
            move || {
                let c = cfg.lock();
                HttpService::build()
                    .keep_alive(c.keep_alive)
                    .client_timeout(c.client_timeout)
                    .finish(factory())
            },
        )?;
        Ok(self)
    }

    #[cfg(feature = "ssl")]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_ssl(
        self,
        lst: net::TcpListener,
        builder: SslAcceptorBuilder,
    ) -> io::Result<Self> {
        self.listen_ssl_inner(lst, openssl_acceptor(builder)?)
    }

    #[cfg(feature = "ssl")]
    fn listen_ssl_inner(
        mut self,
        lst: net::TcpListener,
        acceptor: SslAcceptor,
    ) -> io::Result<Self> {
        use actix_server::ssl::{OpensslAcceptor, SslError};

        let acceptor = OpensslAcceptor::new(acceptor);
        let factory = self.factory.clone();
        let cfg = self.config.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "https",
        });

        self.builder = self.builder.listen(
            format!("actix-web-service-{}", addr),
            lst,
            move || {
                let c = cfg.lock();
                acceptor.clone().map_err(SslError::Ssl).and_then(
                    HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_timeout(c.client_timeout)
                        .client_disconnect(c.client_shutdown)
                        .finish(factory())
                        .map_err(SslError::Service)
                        .map_init_err(|_| ()),
                )
            },
        )?;
        Ok(self)
    }

    #[cfg(feature = "rust-tls")]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_rustls(
        self,
        lst: net::TcpListener,
        config: RustlsServerConfig,
    ) -> io::Result<Self> {
        self.listen_rustls_inner(lst, config)
    }

    #[cfg(feature = "rust-tls")]
    fn listen_rustls_inner(
        mut self,
        lst: net::TcpListener,
        mut config: RustlsServerConfig,
    ) -> io::Result<Self> {
        use actix_server::ssl::{RustlsAcceptor, SslError};

        let protos = vec!["h2".to_string().into(), "http/1.1".to_string().into()];
        config.set_protocols(&protos);

        let acceptor = RustlsAcceptor::new(config);
        let factory = self.factory.clone();
        let cfg = self.config.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "https",
        });

        self.builder = self.builder.listen(
            format!("actix-web-service-{}", addr),
            lst,
            move || {
                let c = cfg.lock();
                acceptor.clone().map_err(SslError::Ssl).and_then(
                    HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_timeout(c.client_timeout)
                        .client_disconnect(c.client_shutdown)
                        .finish(factory())
                        .map_err(SslError::Service)
                        .map_init_err(|_| ()),
                )
            },
        )?;
        Ok(self)
    }

    /// The socket address to bind
    ///
    /// To bind multiple addresses this method can be called multiple times.
    pub fn bind<A: net::ToSocketAddrs>(mut self, addr: A) -> io::Result<Self> {
        let sockets = self.bind2(addr)?;

        for lst in sockets {
            self = self.listen(lst)?;
        }

        Ok(self)
    }

    fn bind2<A: net::ToSocketAddrs>(
        &self,
        addr: A,
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

    #[cfg(feature = "ssl")]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_ssl<A>(
        mut self,
        addr: A,
        builder: SslAcceptorBuilder,
    ) -> io::Result<Self>
    where
        A: net::ToSocketAddrs,
    {
        let sockets = self.bind2(addr)?;
        let acceptor = openssl_acceptor(builder)?;

        for lst in sockets {
            self = self.listen_ssl_inner(lst, acceptor.clone())?;
        }

        Ok(self)
    }

    #[cfg(feature = "rust-tls")]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_rustls<A: net::ToSocketAddrs>(
        mut self,
        addr: A,
        config: RustlsServerConfig,
    ) -> io::Result<Self> {
        let sockets = self.bind2(addr)?;
        for lst in sockets {
            self = self.listen_rustls_inner(lst, config.clone())?;
        }
        Ok(self)
    }

    #[cfg(feature = "uds")]
    /// Start listening for unix domain connections on existing listener.
    ///
    /// This method is available with `uds` feature.
    pub fn listen_uds(
        mut self,
        lst: std::os::unix::net::UnixListener,
    ) -> io::Result<Self> {
        let cfg = self.config.clone();
        let factory = self.factory.clone();
        // todo duplicated:
        self.sockets.push(Socket {
            scheme: "http",
            addr: net::SocketAddr::new(
                net::IpAddr::V4(net::Ipv4Addr::new(127, 0, 0, 1)),
                8080,
            ),
        });

        let addr = format!("actix-web-service-{:?}", lst.local_addr()?);

        self.builder = self.builder.listen_uds(addr, lst, move || {
            let c = cfg.lock();
            HttpService::build()
                .keep_alive(c.keep_alive)
                .client_timeout(c.client_timeout)
                .finish(factory())
        })?;
        Ok(self)
    }

    #[cfg(feature = "uds")]
    /// Start listening for incoming unix domain connections.
    ///
    /// This method is available with `uds` feature.
    pub fn bind_uds<A>(mut self, addr: A) -> io::Result<Self>
    where
        A: AsRef<std::path::Path>,
    {
        let cfg = self.config.clone();
        let factory = self.factory.clone();
        self.sockets.push(Socket {
            scheme: "http",
            addr: net::SocketAddr::new(
                net::IpAddr::V4(net::Ipv4Addr::new(127, 0, 0, 1)),
                8080,
            ),
        });

        self.builder = self.builder.bind_uds(
            format!("actix-web-service-{:?}", addr.as_ref()),
            addr,
            move || {
                let c = cfg.lock();
                HttpService::build()
                    .keep_alive(c.keep_alive)
                    .client_timeout(c.client_timeout)
                    .finish(factory())
            },
        )?;
        Ok(self)
    }
}

impl<F, I, S, B> HttpServer<F, I, S, B>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoNewService<S>,
    S: NewService<Config = ServerConfig, Request = Request>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    S::Service: 'static,
    B: MessageBody,
{
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
    /// use std::io;
    /// use actix_web::{web, App, HttpResponse, HttpServer};
    ///
    /// fn main() -> io::Result<()> {
    ///     let sys = actix_rt::System::new("example");  // <- create Actix system
    ///
    ///     HttpServer::new(|| App::new().service(web::resource("/").to(|| HttpResponse::Ok())))
    ///         .bind("127.0.0.1:0")?
    ///         .start();
    /// #   actix_rt::System::current().stop();
    ///    sys.run()  // <- Run actix system, this method starts all async processes
    /// }
    /// ```
    pub fn start(self) -> Server {
        self.builder.start()
    }

    /// Spawn new thread and start listening for incoming connections.
    ///
    /// This method spawns new thread and starts new actix system. Other than
    /// that it is similar to `start()` method. This method blocks.
    ///
    /// This methods panics if no socket addresses get bound.
    ///
    /// ```rust
    /// use std::io;
    /// use actix_web::{web, App, HttpResponse, HttpServer};
    ///
    /// fn main() -> io::Result<()> {
    /// # std::thread::spawn(|| {
    ///     HttpServer::new(|| App::new().service(web::resource("/").to(|| HttpResponse::Ok())))
    ///         .bind("127.0.0.1:0")?
    ///         .run()
    /// # });
    /// # Ok(())
    /// }
    /// ```
    pub fn run(self) -> io::Result<()> {
        let sys = System::new("http-server");
        self.start();
        sys.run()
    }
}

fn create_tcp_listener(
    addr: net::SocketAddr,
    backlog: i32,
) -> io::Result<net::TcpListener> {
    let builder = match addr {
        net::SocketAddr::V4(_) => TcpBuilder::new_v4()?,
        net::SocketAddr::V6(_) => TcpBuilder::new_v6()?,
    };
    builder.reuse_address(true)?;
    builder.bind(addr)?;
    Ok(builder.listen(backlog)?)
}

#[cfg(feature = "ssl")]
/// Configure `SslAcceptorBuilder` with custom server flags.
fn openssl_acceptor(mut builder: SslAcceptorBuilder) -> io::Result<SslAcceptor> {
    use openssl::ssl::AlpnError;

    builder.set_alpn_select_callback(|_, protos| {
        const H2: &[u8] = b"\x02h2";
        if protos.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else {
            Err(AlpnError::NOACK)
        }
    });
    builder.set_alpn_protos(b"\x08http/1.1\x02h2")?;

    Ok(builder.build())
}

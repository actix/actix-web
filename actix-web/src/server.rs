use std::{
    any::Any,
    cmp, fmt, io,
    marker::PhantomData,
    net,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(feature = "__tls")]
use actix_http::TlsAcceptorConfig;
use actix_http::{body::MessageBody, Extensions, HttpService, KeepAlive, Request, Response};
use actix_server::{Server, ServerBuilder};
use actix_service::{
    map_config, IntoServiceFactory, Service, ServiceFactory, ServiceFactoryExt as _,
};
#[cfg(feature = "openssl")]
use actix_tls::accept::openssl::reexports::{AlpnError, SslAcceptor, SslAcceptorBuilder};

use crate::{config::AppConfig, Error};

struct Socket {
    scheme: &'static str,
    addr: net::SocketAddr,
}

struct Config {
    host: Option<String>,
    keep_alive: KeepAlive,
    client_request_timeout: Duration,
    client_disconnect_timeout: Duration,
    #[allow(dead_code)] // only dead when no TLS features are enabled
    tls_handshake_timeout: Option<Duration>,
}

/// An HTTP Server.
///
/// Create new HTTP server with application factory.
///
/// # Automatic HTTP Version Selection
///
/// There are two ways to select the HTTP version of an incoming connection:
///
/// - One is to rely on the ALPN information that is provided when using a TLS (HTTPS); both
///   versions are supported automatically when using either of the `.bind_rustls()` or
///   `.bind_openssl()` methods.
/// - The other is to read the first few bytes of the TCP stream. This is the only viable approach
///   for supporting H2C, which allows the HTTP/2 protocol to work over plaintext connections. Use
///   the `.bind_auto_h2c()` method to enable this behavior.
///
/// # Examples
///
/// ```no_run
/// use actix_web::{web, App, HttpResponse, HttpServer};
///
/// #[actix_web::main]
/// async fn main() -> std::io::Result<()> {
///     HttpServer::new(|| {
///         App::new()
///             .service(web::resource("/").to(|| async { "hello world" }))
///     })
///     .bind(("127.0.0.1", 8080))?
///     .run()
///     .await
/// }
/// ```
pub struct HttpServer<F, I, S, B>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoServiceFactory<S, Request>,
    S: ServiceFactory<Request, Config = AppConfig>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    B: MessageBody,
{
    pub(super) factory: F,
    config: Arc<Mutex<Config>>,
    backlog: u32,
    sockets: Vec<Socket>,
    builder: ServerBuilder,
    #[allow(clippy::type_complexity)]
    on_connect_fn: Option<Arc<dyn Fn(&dyn Any, &mut Extensions) + Send + Sync>>,
    _phantom: PhantomData<(S, B)>,
}

impl<F, I, S, B> HttpServer<F, I, S, B>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoServiceFactory<S, Request>,

    S: ServiceFactory<Request, Config = AppConfig> + 'static,
    S::Error: Into<Error> + 'static,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>> + 'static,
    <S::Service as Service<Request>>::Future: 'static,
    S::Service: 'static,

    B: MessageBody + 'static,
{
    /// Create new HTTP server with application factory
    ///
    /// # Worker Count
    ///
    /// The `factory` will be instantiated multiple times in most configurations. See
    /// [`bind()`](Self::bind()) docs for more on how worker count and bind address resolution
    /// causes multiple server factory instantiations.
    pub fn new(factory: F) -> Self {
        HttpServer {
            factory,
            config: Arc::new(Mutex::new(Config {
                host: None,
                keep_alive: KeepAlive::default(),
                client_request_timeout: Duration::from_secs(5),
                client_disconnect_timeout: Duration::from_secs(1),
                tls_handshake_timeout: None,
            })),
            backlog: 1024,
            sockets: Vec::new(),
            builder: ServerBuilder::default(),
            on_connect_fn: None,
            _phantom: PhantomData,
        }
    }

    /// Sets number of workers to start (per bind address).
    ///
    /// The default worker count is the determined by [`std::thread::available_parallelism()`]. See
    /// its documentation to determine what behavior you should expect when server is run.
    ///
    /// Note that the server factory passed to [`new`](Self::new()) will be instantiated **at least
    /// once per worker**. See [`bind()`](Self::bind()) docs for more on how worker count and bind
    /// address resolution causes multiple server factory instantiations.
    ///
    /// `num` must be greater than 0.
    ///
    /// # Panics
    ///
    /// Panics if `num` is 0.
    pub fn workers(mut self, num: usize) -> Self {
        self.builder = self.builder.workers(num);
        self
    }

    /// Sets server keep-alive preference.
    ///
    /// By default keep-alive is set to 5 seconds.
    pub fn keep_alive<T: Into<KeepAlive>>(self, val: T) -> Self {
        self.config.lock().unwrap().keep_alive = val.into();
        self
    }

    /// Sets the maximum number of pending connections.
    ///
    /// This refers to the number of clients that can be waiting to be served. Exceeding this number
    /// results in the client getting an error when attempting to connect. It should only affect
    /// servers under significant load.
    ///
    /// Generally set in the 64–2048 range. Default value is 2048.
    ///
    /// This method will have no effect if called after a `bind()`.
    pub fn backlog(mut self, backlog: u32) -> Self {
        self.backlog = backlog;
        self.builder = self.builder.backlog(backlog);
        self
    }

    /// Sets the per-worker maximum number of concurrent connections.
    ///
    /// All socket listeners will stop accepting connections when this limit is reached for
    /// each worker.
    ///
    /// By default max connections is set to a 25k.
    pub fn max_connections(mut self, num: usize) -> Self {
        self.builder = self.builder.max_concurrent_connections(num);
        self
    }

    /// Sets the per-worker maximum concurrent TLS connection limit.
    ///
    /// All listeners will stop accepting connections when this limit is reached. It can be used to
    /// limit the global TLS CPU usage.
    ///
    /// By default max connections is set to a 256.
    #[allow(unused_variables)]
    pub fn max_connection_rate(self, num: usize) -> Self {
        #[cfg(feature = "__tls")]
        actix_tls::accept::max_concurrent_tls_connect(num);
        self
    }

    /// Sets max number of threads for each worker's blocking task thread pool.
    ///
    /// One thread pool is set up **per worker**; not shared across workers.
    ///
    /// By default set to 512 divided by the number of workers.
    pub fn worker_max_blocking_threads(mut self, num: usize) -> Self {
        self.builder = self.builder.worker_max_blocking_threads(num);
        self
    }

    /// Sets server client timeout for first request.
    ///
    /// Defines a timeout for reading client request head. If a client does not transmit the entire
    /// set headers within this time, the request is terminated with a 408 (Request Timeout) error.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default client timeout is set to 5000 milliseconds.
    pub fn client_request_timeout(self, dur: Duration) -> Self {
        self.config.lock().unwrap().client_request_timeout = dur;
        self
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Renamed to `client_request_timeout`.")]
    pub fn client_timeout(self, dur: Duration) -> Self {
        self.client_request_timeout(dur)
    }

    /// Sets server connection shutdown timeout.
    ///
    /// Defines a timeout for connection shutdown. If a shutdown procedure does not complete within
    /// this time, the request is dropped.
    ///
    /// To disable timeout set value to 0.
    ///
    /// By default client timeout is set to 5000 milliseconds.
    pub fn client_disconnect_timeout(self, dur: Duration) -> Self {
        self.config.lock().unwrap().client_disconnect_timeout = dur;
        self
    }

    /// Sets TLS handshake timeout.
    ///
    /// Defines a timeout for TLS handshake. If the TLS handshake does not complete within this
    /// time, the connection is closed.
    ///
    /// By default, the handshake timeout is 3 seconds.
    #[cfg(feature = "__tls")]
    pub fn tls_handshake_timeout(self, dur: Duration) -> Self {
        self.config
            .lock()
            .unwrap()
            .tls_handshake_timeout
            .replace(dur);

        self
    }

    #[doc(hidden)]
    #[deprecated(since = "4.0.0", note = "Renamed to `client_disconnect_timeout`.")]
    pub fn client_shutdown(self, dur: u64) -> Self {
        self.client_disconnect_timeout(Duration::from_millis(dur))
    }

    /// Sets function that will be called once before each connection is handled.
    ///
    /// It will receive a `&std::any::Any`, which contains underlying connection type and an
    /// [Extensions] container so that connection data can be accessed in middleware and handlers.
    ///
    /// # Connection Types
    /// - `actix_tls::accept::openssl::TlsStream<actix_web::rt::net::TcpStream>` when using OpenSSL.
    /// - `actix_tls::accept::rustls_0_20::TlsStream<actix_web::rt::net::TcpStream>` when using
    ///   Rustls v0.20.
    /// - `actix_tls::accept::rustls_0_21::TlsStream<actix_web::rt::net::TcpStream>` when using
    ///   Rustls v0.21.
    /// - `actix_tls::accept::rustls_0_22::TlsStream<actix_web::rt::net::TcpStream>` when using
    ///   Rustls v0.22.
    /// - `actix_tls::accept::rustls_0_23::TlsStream<actix_web::rt::net::TcpStream>` when using
    ///   Rustls v0.23.
    /// - `actix_web::rt::net::TcpStream` when no encryption is used.
    ///
    /// See the `on_connect` example for additional details.
    pub fn on_connect<CB>(self, f: CB) -> HttpServer<F, I, S, B>
    where
        CB: Fn(&dyn Any, &mut Extensions) + Send + Sync + 'static,
    {
        HttpServer {
            factory: self.factory,
            config: self.config,
            backlog: self.backlog,
            sockets: self.sockets,
            builder: self.builder,
            on_connect_fn: Some(Arc::new(f)),
            _phantom: PhantomData,
        }
    }

    /// Sets server host name.
    ///
    /// Host name is used by application router as a hostname for url generation. Check
    /// [`ConnectionInfo`](crate::dev::ConnectionInfo::host()) docs for more info.
    ///
    /// By default, hostname is set to "localhost".
    pub fn server_hostname<T: AsRef<str>>(self, val: T) -> Self {
        self.config.lock().unwrap().host = Some(val.as_ref().to_owned());
        self
    }

    /// Flags the `System` to exit after server shutdown.
    ///
    /// Does nothing when running under `#[tokio::main]` runtime.
    pub fn system_exit(mut self) -> Self {
        self.builder = self.builder.system_exit();
        self
    }

    /// Disables signal handling.
    pub fn disable_signals(mut self) -> Self {
        self.builder = self.builder.disable_signals();
        self
    }

    /// Sets timeout for graceful worker shutdown of workers.
    ///
    /// After receiving a stop signal, workers have this much time to finish serving requests.
    /// Workers still alive after the timeout are force dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, sec: u64) -> Self {
        self.builder = self.builder.shutdown_timeout(sec);
        self
    }

    /// Returns addresses of bound sockets.
    pub fn addrs(&self) -> Vec<net::SocketAddr> {
        self.sockets.iter().map(|s| s.addr).collect()
    }

    /// Returns addresses of bound sockets and the scheme for it.
    ///
    /// This is useful when the server is bound from different sources with some sockets listening
    /// on HTTP and some listening on HTTPS and the user should be presented with an enumeration of
    /// which socket requires which protocol.
    pub fn addrs_with_scheme(&self) -> Vec<(net::SocketAddr, &str)> {
        self.sockets.iter().map(|s| (s.addr, s.scheme)).collect()
    }

    /// Resolves socket address(es) and binds server to created listener(s).
    ///
    /// # Hostname Resolution
    ///
    /// When `addrs` includes a hostname, it is possible for this method to bind to both the IPv4
    /// and IPv6 addresses that result from a DNS lookup. You can test this by passing
    /// `localhost:8080` and noting that the server binds to `127.0.0.1:8080` _and_ `[::1]:8080`. To
    /// bind additional addresses, call this method multiple times.
    ///
    /// Note that, if a DNS lookup is required, resolving hostnames is a blocking operation.
    ///
    /// # Worker Count
    ///
    /// The `factory` will be instantiated multiple times in most scenarios. The number of
    /// instantiations is number of [`workers`](Self::workers()) × number of sockets resolved by
    /// `addrs`.
    ///
    /// For example, if you've manually set [`workers`](Self::workers()) to 2, and use `127.0.0.1`
    /// as the bind `addrs`, then `factory` will be instantiated twice. However, using `localhost`
    /// as the bind `addrs` can often resolve to both `127.0.0.1` (IPv4) _and_ `::1` (IPv6), causing
    /// the `factory` to be instantiated 4 times (2 workers × 2 bind addresses).
    ///
    /// Using a bind address of `0.0.0.0`, which signals to use all interfaces, may also multiple
    /// the number of instantiations in a similar way.
    ///
    /// # Typical Usage
    ///
    /// In general, use `127.0.0.1:<port>` when testing locally and `0.0.0.0:<port>` when deploying
    /// (with or without a reverse proxy or load balancer) so that the server is accessible.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if:
    /// - `addrs` cannot be resolved into one or more socket addresses;
    /// - all the resolved socket addresses are already bound.
    ///
    /// # Example
    ///
    /// ```
    /// # use actix_web::{App, HttpServer};
    /// # fn inner() -> std::io::Result<()> {
    /// HttpServer::new(|| App::new())
    ///     .bind(("127.0.0.1", 8080))?
    ///     .bind("[::1]:9000")?
    /// # ; Ok(()) }
    /// ```
    pub fn bind<A: net::ToSocketAddrs>(mut self, addrs: A) -> io::Result<Self> {
        let sockets = bind_addrs(addrs, self.backlog)?;

        for lst in sockets {
            self = self.listen(lst)?;
        }

        Ok(self)
    }

    /// Resolves socket address(es) and binds server to created listener(s) for plaintext HTTP/1.x
    /// or HTTP/2 connections.
    ///
    /// See [`bind()`](Self::bind()) for more details on `addrs` argument.
    #[cfg(feature = "http2")]
    pub fn bind_auto_h2c<A: net::ToSocketAddrs>(mut self, addrs: A) -> io::Result<Self> {
        let sockets = bind_addrs(addrs, self.backlog)?;

        for lst in sockets {
            self = self.listen_auto_h2c(lst)?;
        }

        Ok(self)
    }

    /// Resolves socket address(es) and binds server to created listener(s) for TLS connections
    /// using Rustls v0.20.
    ///
    /// See [`bind()`](Self::bind()) for more details on `addrs` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_20")]
    pub fn bind_rustls<A: net::ToSocketAddrs>(
        mut self,
        addrs: A,
        config: actix_tls::accept::rustls_0_20::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let sockets = bind_addrs(addrs, self.backlog)?;
        for lst in sockets {
            self = self.listen_rustls_0_20_inner(lst, config.clone())?;
        }
        Ok(self)
    }

    /// Resolves socket address(es) and binds server to created listener(s) for TLS connections
    /// using Rustls v0.21.
    ///
    /// See [`bind()`](Self::bind()) for more details on `addrs` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_21")]
    pub fn bind_rustls_021<A: net::ToSocketAddrs>(
        mut self,
        addrs: A,
        config: actix_tls::accept::rustls_0_21::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let sockets = bind_addrs(addrs, self.backlog)?;
        for lst in sockets {
            self = self.listen_rustls_0_21_inner(lst, config.clone())?;
        }
        Ok(self)
    }

    /// Resolves socket address(es) and binds server to created listener(s) for TLS connections
    /// using Rustls v0.22.
    ///
    /// See [`bind()`](Self::bind()) for more details on `addrs` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_22")]
    pub fn bind_rustls_0_22<A: net::ToSocketAddrs>(
        mut self,
        addrs: A,
        config: actix_tls::accept::rustls_0_22::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let sockets = bind_addrs(addrs, self.backlog)?;
        for lst in sockets {
            self = self.listen_rustls_0_22_inner(lst, config.clone())?;
        }
        Ok(self)
    }

    /// Resolves socket address(es) and binds server to created listener(s) for TLS connections
    /// using Rustls v0.23.
    ///
    /// See [`bind()`](Self::bind()) for more details on `addrs` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_23")]
    pub fn bind_rustls_0_23<A: net::ToSocketAddrs>(
        mut self,
        addrs: A,
        config: actix_tls::accept::rustls_0_23::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let sockets = bind_addrs(addrs, self.backlog)?;
        for lst in sockets {
            self = self.listen_rustls_0_23_inner(lst, config.clone())?;
        }
        Ok(self)
    }

    /// Resolves socket address(es) and binds server to created listener(s) for TLS connections
    /// using OpenSSL.
    ///
    /// See [`bind()`](Self::bind()) for more details on `addrs` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "openssl")]
    pub fn bind_openssl<A>(mut self, addrs: A, builder: SslAcceptorBuilder) -> io::Result<Self>
    where
        A: net::ToSocketAddrs,
    {
        let sockets = bind_addrs(addrs, self.backlog)?;
        let acceptor = openssl_acceptor(builder)?;

        for lst in sockets {
            self = self.listen_openssl_inner(lst, acceptor.clone())?;
        }

        Ok(self)
    }

    /// Binds to existing listener for accepting incoming connection requests.
    ///
    /// No changes are made to `lst`'s configuration. Ensure it is configured properly before
    /// passing ownership to `listen()`.
    pub fn listen(mut self, lst: net::TcpListener) -> io::Result<Self> {
        let cfg = self.config.clone();
        let factory = self.factory.clone();
        let addr = lst.local_addr().unwrap();

        self.sockets.push(Socket {
            addr,
            scheme: "http",
        });

        let on_connect_fn = self.on_connect_fn.clone();

        self.builder =
            self.builder
                .listen(format!("actix-web-service-{}", addr), lst, move || {
                    let cfg = cfg.lock().unwrap();
                    let host = cfg.host.clone().unwrap_or_else(|| format!("{}", addr));

                    let mut svc = HttpService::build()
                        .keep_alive(cfg.keep_alive)
                        .client_request_timeout(cfg.client_request_timeout)
                        .client_disconnect_timeout(cfg.client_disconnect_timeout)
                        .local_addr(addr);

                    if let Some(handler) = on_connect_fn.clone() {
                        svc =
                            svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext))
                    };

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    svc.finish(map_config(fac, move |_| {
                        AppConfig::new(false, host.clone(), addr)
                    }))
                    .tcp()
                })?;

        Ok(self)
    }

    /// Binds to existing listener for accepting incoming plaintext HTTP/1.x or HTTP/2 connections.
    #[cfg(feature = "http2")]
    pub fn listen_auto_h2c(mut self, lst: net::TcpListener) -> io::Result<Self> {
        let cfg = self.config.clone();
        let factory = self.factory.clone();
        let addr = lst.local_addr().unwrap();

        self.sockets.push(Socket {
            addr,
            scheme: "http",
        });

        let on_connect_fn = self.on_connect_fn.clone();

        self.builder =
            self.builder
                .listen(format!("actix-web-service-{}", addr), lst, move || {
                    let cfg = cfg.lock().unwrap();
                    let host = cfg.host.clone().unwrap_or_else(|| format!("{}", addr));

                    let mut svc = HttpService::build()
                        .keep_alive(cfg.keep_alive)
                        .client_request_timeout(cfg.client_request_timeout)
                        .client_disconnect_timeout(cfg.client_disconnect_timeout)
                        .local_addr(addr);

                    if let Some(handler) = on_connect_fn.clone() {
                        svc =
                            svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext))
                    };

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    svc.finish(map_config(fac, move |_| {
                        AppConfig::new(false, host.clone(), addr)
                    }))
                    .tcp_auto_h2c()
                })?;

        Ok(self)
    }

    /// Binds to existing listener for accepting incoming TLS connection requests using Rustls
    /// v0.20.
    ///
    /// See [`listen()`](Self::listen) for more details on the `lst` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_20")]
    pub fn listen_rustls(
        self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_20::reexports::ServerConfig,
    ) -> io::Result<Self> {
        self.listen_rustls_0_20_inner(lst, config)
    }

    /// Binds to existing listener for accepting incoming TLS connection requests using Rustls
    /// v0.21.
    ///
    /// See [`listen()`](Self::listen()) for more details on the `lst` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_21")]
    pub fn listen_rustls_0_21(
        self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_21::reexports::ServerConfig,
    ) -> io::Result<Self> {
        self.listen_rustls_0_21_inner(lst, config)
    }

    #[cfg(feature = "rustls-0_20")]
    fn listen_rustls_0_20_inner(
        mut self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_20::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let factory = self.factory.clone();
        let cfg = self.config.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "https",
        });

        let on_connect_fn = self.on_connect_fn.clone();

        self.builder =
            self.builder
                .listen(format!("actix-web-service-{}", addr), lst, move || {
                    let c = cfg.lock().unwrap();
                    let host = c.host.clone().unwrap_or_else(|| format!("{}", addr));

                    let svc = HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_request_timeout(c.client_request_timeout)
                        .client_disconnect_timeout(c.client_disconnect_timeout);

                    let svc = if let Some(handler) = on_connect_fn.clone() {
                        svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext))
                    } else {
                        svc
                    };

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    let acceptor_config = match c.tls_handshake_timeout {
                        Some(dur) => TlsAcceptorConfig::default().handshake_timeout(dur),
                        None => TlsAcceptorConfig::default(),
                    };

                    svc.finish(map_config(fac, move |_| {
                        AppConfig::new(true, host.clone(), addr)
                    }))
                    .rustls_with_config(config.clone(), acceptor_config)
                })?;

        Ok(self)
    }

    #[cfg(feature = "rustls-0_21")]
    fn listen_rustls_0_21_inner(
        mut self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_21::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let factory = self.factory.clone();
        let cfg = self.config.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "https",
        });

        let on_connect_fn = self.on_connect_fn.clone();

        self.builder =
            self.builder
                .listen(format!("actix-web-service-{}", addr), lst, move || {
                    let c = cfg.lock().unwrap();
                    let host = c.host.clone().unwrap_or_else(|| format!("{}", addr));

                    let svc = HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_request_timeout(c.client_request_timeout)
                        .client_disconnect_timeout(c.client_disconnect_timeout);

                    let svc = if let Some(handler) = on_connect_fn.clone() {
                        svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext))
                    } else {
                        svc
                    };

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    let acceptor_config = match c.tls_handshake_timeout {
                        Some(dur) => TlsAcceptorConfig::default().handshake_timeout(dur),
                        None => TlsAcceptorConfig::default(),
                    };

                    svc.finish(map_config(fac, move |_| {
                        AppConfig::new(true, host.clone(), addr)
                    }))
                    .rustls_021_with_config(config.clone(), acceptor_config)
                })?;

        Ok(self)
    }

    /// Binds to existing listener for accepting incoming TLS connection requests using Rustls
    /// v0.22.
    ///
    /// See [`listen()`](Self::listen()) for more details on the `lst` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_22")]
    pub fn listen_rustls_0_22(
        self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_22::reexports::ServerConfig,
    ) -> io::Result<Self> {
        self.listen_rustls_0_22_inner(lst, config)
    }

    #[cfg(feature = "rustls-0_22")]
    fn listen_rustls_0_22_inner(
        mut self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_22::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let factory = self.factory.clone();
        let cfg = self.config.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "https",
        });

        let on_connect_fn = self.on_connect_fn.clone();

        self.builder =
            self.builder
                .listen(format!("actix-web-service-{}", addr), lst, move || {
                    let c = cfg.lock().unwrap();
                    let host = c.host.clone().unwrap_or_else(|| format!("{}", addr));

                    let svc = HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_request_timeout(c.client_request_timeout)
                        .client_disconnect_timeout(c.client_disconnect_timeout);

                    let svc = if let Some(handler) = on_connect_fn.clone() {
                        svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext))
                    } else {
                        svc
                    };

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    let acceptor_config = match c.tls_handshake_timeout {
                        Some(dur) => TlsAcceptorConfig::default().handshake_timeout(dur),
                        None => TlsAcceptorConfig::default(),
                    };

                    svc.finish(map_config(fac, move |_| {
                        AppConfig::new(true, host.clone(), addr)
                    }))
                    .rustls_0_22_with_config(config.clone(), acceptor_config)
                })?;

        Ok(self)
    }

    /// Binds to existing listener for accepting incoming TLS connection requests using Rustls
    /// v0.23.
    ///
    /// See [`listen()`](Self::listen()) for more details on the `lst` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "rustls-0_23")]
    pub fn listen_rustls_0_23(
        self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_23::reexports::ServerConfig,
    ) -> io::Result<Self> {
        self.listen_rustls_0_23_inner(lst, config)
    }

    #[cfg(feature = "rustls-0_23")]
    fn listen_rustls_0_23_inner(
        mut self,
        lst: net::TcpListener,
        config: actix_tls::accept::rustls_0_23::reexports::ServerConfig,
    ) -> io::Result<Self> {
        let factory = self.factory.clone();
        let cfg = self.config.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "https",
        });

        let on_connect_fn = self.on_connect_fn.clone();

        self.builder =
            self.builder
                .listen(format!("actix-web-service-{}", addr), lst, move || {
                    let c = cfg.lock().unwrap();
                    let host = c.host.clone().unwrap_or_else(|| format!("{}", addr));

                    let svc = HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_request_timeout(c.client_request_timeout)
                        .client_disconnect_timeout(c.client_disconnect_timeout);

                    let svc = if let Some(handler) = on_connect_fn.clone() {
                        svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext))
                    } else {
                        svc
                    };

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    let acceptor_config = match c.tls_handshake_timeout {
                        Some(dur) => TlsAcceptorConfig::default().handshake_timeout(dur),
                        None => TlsAcceptorConfig::default(),
                    };

                    svc.finish(map_config(fac, move |_| {
                        AppConfig::new(true, host.clone(), addr)
                    }))
                    .rustls_0_23_with_config(config.clone(), acceptor_config)
                })?;

        Ok(self)
    }

    /// Binds to existing listener for accepting incoming TLS connection requests using OpenSSL.
    ///
    /// See [`listen()`](Self::listen) for more details on the `lst` argument.
    ///
    /// ALPN protocols "h2" and "http/1.1" are added to any configured ones.
    #[cfg(feature = "openssl")]
    pub fn listen_openssl(
        self,
        lst: net::TcpListener,
        builder: SslAcceptorBuilder,
    ) -> io::Result<Self> {
        self.listen_openssl_inner(lst, openssl_acceptor(builder)?)
    }

    #[cfg(feature = "openssl")]
    fn listen_openssl_inner(
        mut self,
        lst: net::TcpListener,
        acceptor: SslAcceptor,
    ) -> io::Result<Self> {
        let factory = self.factory.clone();
        let cfg = self.config.clone();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            scheme: "https",
        });

        let on_connect_fn = self.on_connect_fn.clone();

        self.builder =
            self.builder
                .listen(format!("actix-web-service-{}", addr), lst, move || {
                    let c = cfg.lock().unwrap();
                    let host = c.host.clone().unwrap_or_else(|| format!("{}", addr));

                    let svc = HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_request_timeout(c.client_request_timeout)
                        .client_disconnect_timeout(c.client_disconnect_timeout)
                        .local_addr(addr);

                    let svc = if let Some(handler) = on_connect_fn.clone() {
                        svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext))
                    } else {
                        svc
                    };

                    let fac = factory()
                        .into_factory()
                        .map_err(|err| err.into().error_response());

                    // false positive lint (?)
                    #[allow(clippy::significant_drop_in_scrutinee)]
                    let acceptor_config = match c.tls_handshake_timeout {
                        Some(dur) => TlsAcceptorConfig::default().handshake_timeout(dur),
                        None => TlsAcceptorConfig::default(),
                    };

                    svc.finish(map_config(fac, move |_| {
                        AppConfig::new(true, host.clone(), addr)
                    }))
                    .openssl_with_config(acceptor.clone(), acceptor_config)
                })?;

        Ok(self)
    }

    /// Opens Unix Domain Socket (UDS) from `uds` path and binds server to created listener.
    #[cfg(unix)]
    pub fn bind_uds<A>(mut self, uds_path: A) -> io::Result<Self>
    where
        A: AsRef<std::path::Path>,
    {
        use actix_http::Protocol;
        use actix_rt::net::UnixStream;
        use actix_service::{fn_service, ServiceFactoryExt as _};

        let cfg = self.config.clone();
        let factory = self.factory.clone();
        let socket_addr =
            net::SocketAddr::new(net::IpAddr::V4(net::Ipv4Addr::new(127, 0, 0, 1)), 8080);

        self.sockets.push(Socket {
            scheme: "http",
            addr: socket_addr,
        });

        self.builder = self.builder.bind_uds(
            format!("actix-web-service-{:?}", uds_path.as_ref()),
            uds_path,
            move || {
                let c = cfg.lock().unwrap();
                let config = AppConfig::new(
                    false,
                    c.host.clone().unwrap_or_else(|| format!("{}", socket_addr)),
                    socket_addr,
                );

                let fac = factory()
                    .into_factory()
                    .map_err(|err| err.into().error_response());

                fn_service(|io: UnixStream| async { Ok((io, Protocol::Http1, None)) }).and_then(
                    HttpService::build()
                        .keep_alive(c.keep_alive)
                        .client_request_timeout(c.client_request_timeout)
                        .client_disconnect_timeout(c.client_disconnect_timeout)
                        .finish(map_config(fac, move |_| config.clone())),
                )
            },
        )?;

        Ok(self)
    }

    /// Binds to existing Unix Domain Socket (UDS) listener.
    #[cfg(unix)]
    pub fn listen_uds(mut self, lst: std::os::unix::net::UnixListener) -> io::Result<Self> {
        use actix_http::Protocol;
        use actix_rt::net::UnixStream;
        use actix_service::{fn_service, ServiceFactoryExt as _};

        let cfg = self.config.clone();
        let factory = self.factory.clone();
        let socket_addr =
            net::SocketAddr::new(net::IpAddr::V4(net::Ipv4Addr::new(127, 0, 0, 1)), 8080);
        self.sockets.push(Socket {
            scheme: "http",
            addr: socket_addr,
        });

        let addr = lst.local_addr()?;
        let name = format!("actix-web-service-{:?}", addr);
        let on_connect_fn = self.on_connect_fn.clone();

        self.builder = self.builder.listen_uds(name, lst, move || {
            let c = cfg.lock().unwrap();
            let config = AppConfig::new(
                false,
                c.host.clone().unwrap_or_else(|| format!("{}", socket_addr)),
                socket_addr,
            );

            fn_service(|io: UnixStream| async { Ok((io, Protocol::Http1, None)) }).and_then({
                let mut svc = HttpService::build()
                    .keep_alive(c.keep_alive)
                    .client_request_timeout(c.client_request_timeout)
                    .client_disconnect_timeout(c.client_disconnect_timeout);

                if let Some(handler) = on_connect_fn.clone() {
                    svc = svc.on_connect_ext(move |io: &_, ext: _| (handler)(io as &dyn Any, ext));
                }

                let fac = factory()
                    .into_factory()
                    .map_err(|err| err.into().error_response());

                svc.finish(map_config(fac, move |_| config.clone()))
            })
        })?;
        Ok(self)
    }
}

impl<F, I, S, B> HttpServer<F, I, S, B>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: IntoServiceFactory<S, Request>,
    S: ServiceFactory<Request, Config = AppConfig>,
    S::Error: Into<Error>,
    S::InitError: fmt::Debug,
    S::Response: Into<Response<B>>,
    S::Service: 'static,
    B: MessageBody,
{
    /// Start listening for incoming connections.
    ///
    /// # Workers
    /// This method starts a number of HTTP workers in separate threads. The number of workers in a
    /// set is defined by [`workers()`](Self::workers) or, by default, the number of the machine's
    /// physical cores. One worker set is created for each socket address to be bound. For example,
    /// if workers is set to 4, and there are 2 addresses to bind, then 8 worker threads will be
    /// spawned.
    ///
    /// # Panics
    /// This methods panics if no socket addresses were successfully bound or if no Tokio runtime
    /// is set up.
    pub fn run(self) -> Server {
        self.builder.run()
    }
}

/// Bind TCP listeners to socket addresses resolved from `addrs` with options.
fn bind_addrs(addrs: impl net::ToSocketAddrs, backlog: u32) -> io::Result<Vec<net::TcpListener>> {
    let mut err = None;
    let mut success = false;
    let mut sockets = Vec::new();

    for addr in addrs.to_socket_addrs()? {
        match create_tcp_listener(addr, backlog) {
            Ok(lst) => {
                success = true;
                sockets.push(lst);
            }
            Err(error) => err = Some(error),
        }
    }

    if success {
        Ok(sockets)
    } else if let Some(err) = err.take() {
        Err(err)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Can not bind to address.",
        ))
    }
}

/// Creates a TCP listener from socket address and options.
fn create_tcp_listener(addr: net::SocketAddr, backlog: u32) -> io::Result<net::TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let domain = Domain::for_address(addr);
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    // clamp backlog to max u32 that fits in i32 range
    let backlog = cmp::min(backlog, i32::MAX as u32) as i32;
    socket.listen(backlog)?;
    Ok(net::TcpListener::from(socket))
}

/// Configures OpenSSL acceptor `builder` with ALPN protocols.
#[cfg(feature = "openssl")]
fn openssl_acceptor(mut builder: SslAcceptorBuilder) -> io::Result<SslAcceptor> {
    builder.set_alpn_select_callback(|_, protocols| {
        const H2: &[u8] = b"\x02h2";
        const H11: &[u8] = b"\x08http/1.1";

        if protocols.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else if protocols.windows(9).any(|window| window == H11) {
            Ok(b"http/1.1")
        } else {
            Err(AlpnError::NOACK)
        }
    });

    builder.set_alpn_protos(b"\x08http/1.1\x02h2")?;

    Ok(builder.build())
}

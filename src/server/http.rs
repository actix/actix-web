use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::{io, mem, net, time};

use actix::{Actor, Addr, Arbiter, AsyncContext, Context, Handler, System};

use futures::{Future, Stream};
use net2::{TcpBuilder, TcpStreamExt};
use num_cpus;
use tokio::executor::current_thread;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tcp::TcpStream;

#[cfg(feature = "tls")]
use native_tls::TlsAcceptor;

#[cfg(feature = "alpn")]
use openssl::ssl::SslAcceptorBuilder;

#[cfg(feature = "rust-tls")]
use rustls::ServerConfig;

use super::channel::{HttpChannel, WrapperStream};
use super::server::{Connections, Server, Service, ServiceHandler};
use super::settings::{ServerSettings, WorkerSettings};
use super::worker::{Conn, Socket};
use super::{
    AcceptorService, HttpHandler, IntoAsyncIo, IntoHttpHandler, IoStream, KeepAlive,
    Token,
};

/// An HTTP Server
pub struct HttpServer<H>
where
    H: IntoHttpHandler + 'static,
{
    factory: Arc<Fn() -> Vec<H> + Send + Sync>,
    host: Option<String>,
    keep_alive: KeepAlive,
    backlog: i32,
    threads: usize,
    exit: bool,
    shutdown_timeout: u16,
    no_http2: bool,
    no_signals: bool,
    maxconn: usize,
    maxconnrate: usize,
    sockets: Vec<Socket>,
    handlers: Vec<Box<IoStreamHandler<H::Handler, net::TcpStream>>>,
}

impl<H> HttpServer<H>
where
    H: IntoHttpHandler + 'static,
{
    /// Create new http server with application factory
    pub fn new<F, U>(factory: F) -> Self
    where
        F: Fn() -> U + Sync + Send + 'static,
        U: IntoIterator<Item = H> + 'static,
    {
        let f = move || (factory)().into_iter().collect();

        HttpServer {
            threads: num_cpus::get(),
            factory: Arc::new(f),
            host: None,
            backlog: 2048,
            keep_alive: KeepAlive::Os,
            shutdown_timeout: 30,
            exit: true,
            no_http2: false,
            no_signals: false,
            maxconn: 102_400,
            maxconnrate: 256,
            // settings: None,
            sockets: Vec::new(),
            handlers: Vec::new(),
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
    /// By default max connections is set to a 100k.
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
    /// By default keep alive is set to a `Os`.
    pub fn keep_alive<T: Into<KeepAlive>>(mut self, val: T) -> Self {
        self.keep_alive = val.into();
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
    #[doc(hidden)]
    #[deprecated(
        since = "0.7.4",
        note = "please use acceptor service with proper ServerFlags parama"
    )]
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
        self.handlers
            .iter()
            .map(|s| (s.addr(), s.scheme()))
            .collect()
    }

    /// Use listener for accepting incoming connection requests
    ///
    /// HttpServer does not change any configuration for TcpListener,
    /// it needs to be configured before passing it to listen() method.
    pub fn listen(mut self, lst: net::TcpListener) -> Self {
        let token = Token(self.handlers.len());
        let addr = lst.local_addr().unwrap();
        self.handlers
            .push(Box::new(SimpleHandler::new(lst.local_addr().unwrap())));
        self.sockets.push(Socket { lst, addr, token });

        self
    }

    /// Use listener for accepting incoming connection requests
    pub fn listen_with<A>(mut self, lst: net::TcpListener, acceptor: A) -> Self
    where
        A: AcceptorService<TcpStream> + Send + 'static,
    {
        let token = Token(self.handlers.len());
        let addr = lst.local_addr().unwrap();
        self.handlers.push(Box::new(StreamHandler::new(
            lst.local_addr().unwrap(),
            acceptor,
        )));
        self.sockets.push(Socket { lst, addr, token });

        self
    }

    #[cfg(feature = "tls")]
    #[doc(hidden)]
    #[deprecated(
        since = "0.7.4",
        note = "please use `actix_web::HttpServer::listen_with()` and `actix_web::server::NativeTlsAcceptor` instead"
    )]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// HttpServer does not change any configuration for TcpListener,
    /// it needs to be configured before passing it to listen() method.
    pub fn listen_tls(self, lst: net::TcpListener, acceptor: TlsAcceptor) -> Self {
        use super::NativeTlsAcceptor;

        self.listen_with(lst, NativeTlsAcceptor::new(acceptor))
    }

    #[cfg(feature = "alpn")]
    #[doc(hidden)]
    #[deprecated(
        since = "0.7.4",
        note = "please use `actix_web::HttpServer::listen_with()` and `actix_web::server::OpensslAcceptor` instead"
    )]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_ssl(
        self, lst: net::TcpListener, builder: SslAcceptorBuilder,
    ) -> io::Result<Self> {
        use super::{OpensslAcceptor, ServerFlags};

        // alpn support
        let flags = if self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        Ok(self.listen_with(lst, OpensslAcceptor::with_flags(builder, flags)?))
    }

    #[cfg(feature = "rust-tls")]
    #[doc(hidden)]
    #[deprecated(
        since = "0.7.4",
        note = "please use `actix_web::HttpServer::listen_with()` and `actix_web::server::RustlsAcceptor` instead"
    )]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_rustls(self, lst: net::TcpListener, builder: ServerConfig) -> Self {
        use super::{RustlsAcceptor, ServerFlags};

        // alpn support
        let flags = if self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        self.listen_with(lst, RustlsAcceptor::with_flags(builder, flags))
    }

    /// The socket address to bind
    ///
    /// To bind multiple addresses this method can be called multiple times.
    pub fn bind<S: net::ToSocketAddrs>(mut self, addr: S) -> io::Result<Self> {
        let sockets = self.bind2(addr)?;

        for lst in sockets {
            let token = Token(self.handlers.len());
            let addr = lst.local_addr().unwrap();
            self.handlers
                .push(Box::new(SimpleHandler::new(lst.local_addr().unwrap())));
            self.sockets.push(Socket { lst, addr, token })
        }

        Ok(self)
    }

    /// Start listening for incoming connections with supplied acceptor.
    #[cfg_attr(feature = "cargo-clippy", allow(needless_pass_by_value))]
    pub fn bind_with<S, A>(mut self, addr: S, acceptor: A) -> io::Result<Self>
    where
        S: net::ToSocketAddrs,
        A: AcceptorService<TcpStream> + Send + 'static,
    {
        let sockets = self.bind2(addr)?;

        for lst in sockets {
            let token = Token(self.handlers.len());
            let addr = lst.local_addr().unwrap();
            self.handlers.push(Box::new(StreamHandler::new(
                lst.local_addr().unwrap(),
                acceptor.clone(),
            )));
            self.sockets.push(Socket { lst, addr, token })
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
    #[doc(hidden)]
    #[deprecated(
        since = "0.7.4",
        note = "please use `actix_web::HttpServer::bind_with()` and `actix_web::server::NativeTlsAcceptor` instead"
    )]
    /// The ssl socket address to bind
    ///
    /// To bind multiple addresses this method can be called multiple times.
    pub fn bind_tls<S: net::ToSocketAddrs>(
        self, addr: S, acceptor: TlsAcceptor,
    ) -> io::Result<Self> {
        use super::NativeTlsAcceptor;

        self.bind_with(addr, NativeTlsAcceptor::new(acceptor))
    }

    #[cfg(feature = "alpn")]
    #[doc(hidden)]
    #[deprecated(
        since = "0.7.4",
        note = "please use `actix_web::HttpServer::bind_with()` and `actix_web::server::OpensslAcceptor` instead"
    )]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_ssl<S>(self, addr: S, builder: SslAcceptorBuilder) -> io::Result<Self>
    where
        S: net::ToSocketAddrs,
    {
        use super::{OpensslAcceptor, ServerFlags};

        // alpn support
        let flags = if !self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        self.bind_with(addr, OpensslAcceptor::with_flags(builder, flags)?)
    }

    #[cfg(feature = "rust-tls")]
    #[doc(hidden)]
    #[deprecated(
        since = "0.7.4",
        note = "please use `actix_web::HttpServer::bind_with()` and `actix_web::server::RustlsAcceptor` instead"
    )]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_rustls<S: net::ToSocketAddrs>(
        self, addr: S, builder: ServerConfig,
    ) -> io::Result<Self> {
        use super::{RustlsAcceptor, ServerFlags};

        // alpn support
        let flags = if !self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        self.bind_with(addr, RustlsAcceptor::with_flags(builder, flags))
    }
}

impl<H: IntoHttpHandler> Into<(Box<Service>, Vec<(Token, net::TcpListener)>)> for HttpServer<H> {
    fn into(mut self) -> (Box<Service>, Vec<(Token, net::TcpListener)>) {
        let sockets: Vec<_> = mem::replace(&mut self.sockets, Vec::new())
            .into_iter()
            .map(|item| (item.token, item.lst))
            .collect();

        (Box::new(HttpService {
            factory: self.factory,
            host: self.host,
            keep_alive: self.keep_alive,
            handlers: self.handlers,
        }), sockets)
    }
}

struct HttpService<H: IntoHttpHandler> {
    factory: Arc<Fn() -> Vec<H> + Send + Sync>,
    host: Option<String>,
    keep_alive: KeepAlive,
    handlers: Vec<Box<IoStreamHandler<H::Handler, net::TcpStream>>>,
}

impl<H: IntoHttpHandler + 'static> Service for HttpService<H> {
    fn clone(&self) -> Box<Service> {
        Box::new(HttpService {
            factory: self.factory.clone(),
            host: self.host.clone(),
            keep_alive: self.keep_alive,
            handlers: self.handlers.iter().map(|v| v.clone()).collect(),
        })
    }

    fn create(&self, conns: Connections) -> Box<ServiceHandler> {
        let addr = self.handlers[0].addr();
        let s = ServerSettings::new(Some(addr), &self.host, false);
        let apps: Vec<_> = (*self.factory)()
            .into_iter()
            .map(|h| h.into_handler())
            .collect();
        let handlers = self.handlers.iter().map(|h| h.clone()).collect();

        Box::new(HttpServiceHandler::new(
            apps,
            handlers,
            self.keep_alive,
            s,
            conns,
        ))
    }
}

impl<H: IntoHttpHandler> HttpServer<H> {
    /// Start listening for incoming connections.
    ///
    /// This method starts number of http workers in separate threads.
    /// For each address this method starts separate thread which does
    /// `accept()` in a loop.
    ///
    /// This methods panics if no socket addresses get bound.
    ///
    /// This method requires to run within properly configured `Actix` system.
    ///
    /// ```rust
    /// extern crate actix_web;
    /// use actix_web::{actix, server, App, HttpResponse};
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
    pub fn start(self) -> Addr<Server> {
        let mut srv = Server::new()
            .workers(self.threads)
            .maxconn(self.maxconn)
            .maxconnrate(self.maxconnrate)
            .shutdown_timeout(self.shutdown_timeout);

        srv = if self.exit { srv.system_exit() } else { srv };
        srv = if self.no_signals {
            srv.disable_signals()
        } else {
            srv
        };

        srv.service(self).start()
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
}

impl<H: IntoHttpHandler> HttpServer<H> {
    /// Start listening for incoming connections from a stream.
    ///
    /// This method uses only one thread for handling incoming connections.
    pub fn start_incoming<T, S>(self, stream: S, secure: bool)
    where
        S: Stream<Item = T, Error = io::Error> + Send + 'static,
        T: AsyncRead + AsyncWrite + Send + 'static,
    {
        // set server settings
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let srv_settings = ServerSettings::new(Some(addr), &self.host, secure);
        let apps: Vec<_> = (*self.factory)()
            .into_iter()
            .map(|h| h.into_handler())
            .collect();
        let settings = WorkerSettings::create(
            apps,
            self.keep_alive,
            srv_settings,
            Connections::default(),
        );

        // start server
        HttpIncoming::create(move |ctx| {
            ctx.add_message_stream(stream.map_err(|_| ()).map(move |t| Conn {
                io: WrapperStream::new(t),
                handler: Token::new(0),
                token: Token::new(0),
                peer: None,
            }));
            HttpIncoming { settings }
        });
    }
}

struct HttpIncoming<H: HttpHandler> {
    settings: Rc<WorkerSettings<H>>,
}

impl<H> Actor for HttpIncoming<H>
where
    H: HttpHandler,
{
    type Context = Context<Self>;
}

impl<T, H> Handler<Conn<T>> for HttpIncoming<H>
where
    T: IoStream,
    H: HttpHandler,
{
    type Result = ();

    fn handle(&mut self, msg: Conn<T>, _: &mut Context<Self>) -> Self::Result {
        Arbiter::spawn(HttpChannel::new(
            Rc::clone(&self.settings),
            msg.io,
            msg.peer,
        ));
    }
}

struct HttpServiceHandler<H>
where
    H: HttpHandler + 'static,
{
    settings: Rc<WorkerSettings<H>>,
    handlers: Vec<Box<IoStreamHandler<H, net::TcpStream>>>,
    tcp_ka: Option<time::Duration>,
}

impl<H: HttpHandler + 'static> HttpServiceHandler<H> {
    fn new(
        apps: Vec<H>, handlers: Vec<Box<IoStreamHandler<H, net::TcpStream>>>,
        keep_alive: KeepAlive, settings: ServerSettings, conns: Connections,
    ) -> HttpServiceHandler<H> {
        let tcp_ka = if let KeepAlive::Tcp(val) = keep_alive {
            Some(time::Duration::new(val as u64, 0))
        } else {
            None
        };
        let settings = WorkerSettings::create(apps, keep_alive, settings, conns);

        HttpServiceHandler {
            handlers,
            tcp_ka,
            settings,
        }
    }
}

impl<H> ServiceHandler for HttpServiceHandler<H>
where
    H: HttpHandler + 'static,
{
    fn handle(
        &mut self, token: Token, io: net::TcpStream, peer: Option<net::SocketAddr>,
    ) {
        if self.tcp_ka.is_some() && io.set_keepalive(self.tcp_ka).is_err() {
            error!("Can not set socket keep-alive option");
        }
        self.handlers[token.0].handle(Rc::clone(&self.settings), io, peer);
    }

    fn shutdown(&self, force: bool) {
        if force {
            self.settings.head().traverse::<TcpStream, H>();
        }
    }
}

struct SimpleHandler<Io> {
    addr: net::SocketAddr,
    io: PhantomData<Io>,
}

impl<Io: IntoAsyncIo> Clone for SimpleHandler<Io> {
    fn clone(&self) -> Self {
        SimpleHandler {
            addr: self.addr,
            io: PhantomData,
        }
    }
}

impl<Io: IntoAsyncIo> SimpleHandler<Io> {
    fn new(addr: net::SocketAddr) -> Self {
        SimpleHandler {
            addr,
            io: PhantomData,
        }
    }
}

impl<H, Io> IoStreamHandler<H, Io> for SimpleHandler<Io>
where
    H: HttpHandler,
    Io: IntoAsyncIo + Send + 'static,
    Io::Io: IoStream,
{
    fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    fn clone(&self) -> Box<IoStreamHandler<H, Io>> {
        Box::new(Clone::clone(self))
    }

    fn scheme(&self) -> &'static str {
        "http"
    }

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>) {
        let mut io = match io.into_async_io() {
            Ok(io) => io,
            Err(err) => {
                trace!("Failed to create async io: {}", err);
                return;
            }
        };
        let _ = io.set_nodelay(true);

        current_thread::spawn(HttpChannel::new(h, io, peer));
    }
}

struct StreamHandler<A, Io> {
    acceptor: A,
    addr: net::SocketAddr,
    io: PhantomData<Io>,
}

impl<Io: IntoAsyncIo, A: AcceptorService<Io::Io>> StreamHandler<A, Io> {
    fn new(addr: net::SocketAddr, acceptor: A) -> Self {
        StreamHandler {
            addr,
            acceptor,
            io: PhantomData,
        }
    }
}

impl<Io: IntoAsyncIo, A: AcceptorService<Io::Io>> Clone for StreamHandler<A, Io> {
    fn clone(&self) -> Self {
        StreamHandler {
            addr: self.addr,
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<H, Io, A> IoStreamHandler<H, Io> for StreamHandler<A, Io>
where
    H: HttpHandler,
    Io: IntoAsyncIo + Send + 'static,
    Io::Io: IoStream,
    A: AcceptorService<Io::Io> + Send + 'static,
{
    fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    fn clone(&self) -> Box<IoStreamHandler<H, Io>> {
        Box::new(Clone::clone(self))
    }

    fn scheme(&self) -> &'static str {
        self.acceptor.scheme()
    }

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>) {
        let mut io = match io.into_async_io() {
            Ok(io) => io,
            Err(err) => {
                trace!("Failed to create async io: {}", err);
                return;
            }
        };
        let _ = io.set_nodelay(true);

        let rate = h.connection_rate();
        current_thread::spawn(self.acceptor.accept(io).then(move |res| {
            drop(rate);
            match res {
                Ok(io) => current_thread::spawn(HttpChannel::new(h, io, peer)),
                Err(err) => trace!("Can not establish connection: {}", err),
            }
            Ok(())
        }))
    }
}

impl<H, Io: 'static> IoStreamHandler<H, Io> for Box<IoStreamHandler<H, Io>>
where
    H: HttpHandler,
    Io: IntoAsyncIo,
{
    fn addr(&self) -> net::SocketAddr {
        self.as_ref().addr()
    }

    fn clone(&self) -> Box<IoStreamHandler<H, Io>> {
        self.as_ref().clone()
    }

    fn scheme(&self) -> &'static str {
        self.as_ref().scheme()
    }

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>) {
        self.as_ref().handle(h, io, peer)
    }
}

trait IoStreamHandler<H, Io>: Send
where
    H: HttpHandler,
{
    fn clone(&self) -> Box<IoStreamHandler<H, Io>>;

    fn addr(&self) -> net::SocketAddr;

    fn scheme(&self) -> &'static str;

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>);
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

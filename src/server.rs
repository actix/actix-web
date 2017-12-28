use std::{io, net, thread};
use std::rc::Rc;
use std::cell::{RefCell, RefMut};
use std::sync::{Arc, mpsc as sync_mpsc};
use std::time::Duration;
use std::marker::PhantomData;
use std::collections::HashMap;

use actix::dev::*;
use actix::System;
use futures::Stream;
use futures::sync::mpsc;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::reactor::Handle;
use tokio_core::net::TcpStream;
use mio;
use num_cpus;
use net2::{TcpBuilder, TcpStreamExt};

#[cfg(feature="tls")]
use futures::{future, Future};
#[cfg(feature="tls")]
use native_tls::TlsAcceptor;
#[cfg(feature="tls")]
use tokio_tls::{TlsStream, TlsAcceptorExt};

#[cfg(feature="alpn")]
use futures::{future, Future};
#[cfg(feature="alpn")]
use openssl::ssl::{SslMethod, SslAcceptor, SslAcceptorBuilder};
#[cfg(feature="alpn")]
use openssl::pkcs12::ParsedPkcs12;
#[cfg(feature="alpn")]
use tokio_openssl::{SslStream, SslAcceptorExt};

#[cfg(feature="signal")]
use actix::actors::signal;

use helpers;
use channel::{HttpChannel, HttpHandler, IntoHttpHandler};

/// Various server settings
#[derive(Debug, Clone)]
pub struct ServerSettings {
    addr: Option<net::SocketAddr>,
    secure: bool,
    host: String,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            addr: None,
            secure: false,
            host: "localhost:8080".to_owned(),
        }
    }
}

impl ServerSettings {
    /// Crate server settings instance
    fn new(addr: Option<net::SocketAddr>, host: &Option<String>, secure: bool) -> Self {
        let host = if let Some(ref host) = *host {
            host.clone()
        } else if let Some(ref addr) = addr {
            format!("{}", addr)
        } else {
            "localhost".to_owned()
        };
        ServerSettings {
            addr: addr,
            secure: secure,
            host: host,
        }
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
        self.addr
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns host header value
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// An HTTP Server
///
/// `T` - async stream, anything that implements `AsyncRead` + `AsyncWrite`.
///
/// `A` - peer address
///
/// `H` - request handler
pub struct HttpServer<T, A, H, U>
    where H: 'static
{
    h: Option<Rc<WorkerSettings<H>>>,
    io: PhantomData<T>,
    addr: PhantomData<A>,
    threads: usize,
    backlog: i32,
    host: Option<String>,
    keep_alive: Option<u64>,
    factory: Arc<Fn() -> U + Send + Sync>,
    workers: Vec<SyncAddress<Worker<H>>>,
    sockets: HashMap<net::SocketAddr, net::TcpListener>,
    accept: Vec<(mio::SetReadiness, sync_mpsc::Sender<Command>)>,
    exit: bool,
}

unsafe impl<T, A, H, U> Sync for HttpServer<T, A, H, U> where H: 'static {}
unsafe impl<T, A, H, U> Send for HttpServer<T, A, H, U> where H: 'static {}


impl<T: 'static, A: 'static, H, U: 'static> Actor for HttpServer<T, A, H, U> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<T: 'static, A: 'static, H, U: 'static>  HttpServer<T, A, H, U> {
    fn update_time(&self, ctx: &mut Context<Self>) {
        helpers::update_date();
        ctx.run_later(Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }
}

impl<T, A, H, U, V> HttpServer<T, A, H, U>
    where A: 'static,
          T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler,
          U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Create new http server with application factory
    pub fn new<F>(factory: F) -> Self
        where F: Sync + Send + 'static + Fn() -> U,
    {
        HttpServer{ h: None,
                    io: PhantomData,
                    addr: PhantomData,
                    threads: num_cpus::get(),
                    backlog: 2048,
                    host: None,
                    keep_alive: None,
                    factory: Arc::new(factory),
                    workers: Vec::new(),
                    sockets: HashMap::new(),
                    accept: Vec::new(),
                    exit: false,
        }
    }

    /// Set number of workers to start.
    ///
    /// By default http server uses number of available logical cpu as threads count.
    pub fn threads(mut self, num: usize) -> Self {
        self.threads = num;
        self
    }

    /// Set the maximum number of pending connections.
    ///
    /// This refers to the number of clients that can be waiting to be served.
    /// Exceeding this number results in the client getting an error when
    /// attempting to connect. It should only affect servers under significant load.
    ///
    /// Generally set in the 64-2048 range. Default value is 2048.
    ///
    /// This method should be called before `bind()` method call.
    pub fn backlog(mut self, num: i32) -> Self {
        self.backlog = num;
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is enabled.
    ///
    ///  - `Some(75)` - enable
    ///
    ///  - `Some(0)` - disable
    ///
    ///  - `None` - use `SO_KEEPALIVE` socket option
    pub fn keep_alive(mut self, val: Option<u64>) -> Self {
        self.keep_alive = val;
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url generation.
    /// Check [ConnectionInfo](./dev/struct.ConnectionInfo.html#method.host) documentation
    /// for more information.
    pub fn server_hostname(mut self, val: String) -> Self {
        self.host = Some(val);
        self
    }

    #[cfg(feature="signal")]
    /// Send `SystemExit` message to actix system
    ///
    /// `SystemExit` message stops currently running system arbiter and all
    /// nested arbiters.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    /// Get addresses of bound sockets.
    pub fn addrs(&self) -> Vec<net::SocketAddr> {
        self.sockets.keys().cloned().collect()
    }

    /// The socket address to bind
    ///
    /// To mind multiple addresses this method can be call multiple times.
    pub fn bind<S: net::ToSocketAddrs>(mut self, addr: S) -> io::Result<Self> {
        let mut err = None;
        let mut succ = false;
        for addr in addr.to_socket_addrs()? {
            match create_tcp_listener(addr, self.backlog) {
                Ok(lst) => {
                    succ = true;
                    self.sockets.insert(lst.local_addr().unwrap(), lst);
                },
                Err(e) => err = Some(e),
            }
        }

        if !succ {
            if let Some(e) = err.take() {
                Err(e)
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "Can not bind to address."))
            }
        } else {
            Ok(self)
        }
    }

    fn start_workers(&mut self, settings: &ServerSettings, handler: &StreamHandlerType)
                     -> Vec<mpsc::UnboundedSender<IoStream<net::TcpStream>>>
    {
        // start workers
        let mut workers = Vec::new();
        for _ in 0..self.threads {
            let s = settings.clone();
            let (tx, rx) = mpsc::unbounded::<IoStream<net::TcpStream>>();

            let h = handler.clone();
            let ka = self.keep_alive;
            let factory = Arc::clone(&self.factory);
            let addr = Arbiter::start(move |ctx: &mut Context<_>| {
                let mut apps: Vec<_> = (*factory)()
                    .into_iter().map(|h| h.into_handler()).collect();
                for app in &mut apps {
                    app.server_settings(s.clone());
                }
                ctx.add_stream(rx);
                Worker::new(apps, h, ka)
            });
            workers.push(tx);
            self.workers.push(addr);
        }
        info!("Starting {} http workers", self.threads);
        workers
    }
}

impl<H: HttpHandler, U, V> HttpServer<TcpStream, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming connections.
    ///
    /// This method starts number of http handler workers in seperate threads.
    /// For each address this method starts separate thread which does `accept()` in a loop.
    ///
    /// This methods panics if no socket addresses get bound.
    ///
    /// This method requires to run within properly configured `Actix` system.
    ///
    /// ```rust
    /// extern crate actix;
    /// extern crate actix_web;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let sys = actix::System::new("example");  // <- create Actix system
    ///
    ///     HttpServer::new(
    ///         || Application::new()
    ///              .resource("/", |r| r.h(httpcodes::HTTPOk)))
    ///         .bind("127.0.0.1:0").expect("Can not bind to 127.0.0.1:0")
    ///         .start();
    /// #  actix::Arbiter::system().send(actix::msgs::SystemExit(0));
    ///
    ///    let _ = sys.run();  // <- Run actix system, this method actually starts all async processes
    /// }
    /// ```
    pub fn start(mut self) -> SyncAddress<Self>
    {
        if self.sockets.is_empty() {
            panic!("HttpServer::bind() has to be called befor start()");
        } else {
            let addrs: Vec<(net::SocketAddr, net::TcpListener)> =
                self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), &self.host, false);
            let workers = self.start_workers(&settings, &StreamHandlerType::Normal);

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting http server on {}", addr);
                self.accept.push(
                    start_accept_thread(sock, addr, self.backlog, workers.clone()));
            }

            // start http server actor
            HttpServer::create(|_| {self})
        }
    }

    /// Spawn new thread and start listening for incomming connections.
    ///
    /// This method spawns new thread and starts new actix system. Other than that it is
    /// similar to `start()` method. This method does not block.
    ///
    /// This methods panics if no socket addresses get bound.
    ///
    /// ```rust
    /// # extern crate futures;
    /// # extern crate actix;
    /// # extern crate actix_web;
    /// # use futures::Future;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     let addr = HttpServer::new(
    ///         || Application::new()
    ///              .resource("/", |r| r.h(httpcodes::HTTPOk)))
    ///         .bind("127.0.0.1:0").expect("Can not bind to 127.0.0.1:0")
    ///         .spawn();
    ///
    ///     let _ = addr.call_fut(
    ///           dev::StopServer{graceful:true}).wait();  // <- Send `StopServer` message to server.
    /// }
    /// ```
    pub fn spawn(mut self) -> SyncAddress<Self> {
        self.exit = true;

        let (tx, rx) = sync_mpsc::channel();
        thread::spawn(move || {
            let sys = System::new("http-server");
            let addr = self.start();
            let _ = tx.send(addr);
            sys.run();
        });
        rx.recv().unwrap()
    }
}

#[cfg(feature="tls")]
impl<H: HttpHandler, U, V> HttpServer<TlsStream<TcpStream>, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming tls connections.
    pub fn start_tls(mut self, pkcs12: ::Pkcs12) -> io::Result<SyncAddress<Self>> {
        if self.sockets.is_empty() {
            Err(io::Error::new(io::ErrorKind::Other, "No socket addresses are bound"))
        } else {
            let addrs: Vec<(net::SocketAddr, net::TcpListener)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), &self.host, false);
            let acceptor = match TlsAcceptor::builder(pkcs12) {
                Ok(builder) => {
                    match builder.build() {
                        Ok(acceptor) => acceptor,
                        Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
                    }
                }
                Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
            };
            let workers = self.start_workers(&settings, &StreamHandlerType::Tls(acceptor));

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting tls http server on {}", addr);
                self.accept.push(
                    start_accept_thread(sock, addr, self.backlog, workers.clone()));
            }

            // start http server actor
            Ok(HttpServer::create(|_| {self}))
        }
    }
}

#[cfg(feature="alpn")]
impl<H: HttpHandler, U, V> HttpServer<SslStream<TcpStream>, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn start_ssl(mut self, identity: &ParsedPkcs12) -> io::Result<SyncAddress<Self>> {
        if self.sockets.is_empty() {
            Err(io::Error::new(io::ErrorKind::Other, "No socket addresses are bound"))
        } else {
            let addrs: Vec<(net::SocketAddr, net::TcpListener)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), &self.host, false);
            let acceptor = match SslAcceptorBuilder::mozilla_intermediate(
                SslMethod::tls(), &identity.pkey, &identity.cert, &identity.chain)
            {
                Ok(mut builder) => {
                    match builder.set_alpn_protocols(&[b"h2", b"http/1.1"]) {
                        Ok(_) => builder.build(),
                        Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err)),
                    }
                },
                Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
            };
            let workers = self.start_workers(&settings, &StreamHandlerType::Alpn(acceptor));

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting tls http server on {}", addr);
                self.accept.push(
                    start_accept_thread(sock, addr, self.backlog, workers.clone()));
            }

            // start http server actor
            Ok(HttpServer::create(|_| {self}))
        }
    }
}

impl<T, A, H, U, V> HttpServer<T, A, H, U>
    where A: 'static,
          T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler,
          U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming connections from a stream.
    ///
    /// This method uses only one thread for handling incoming connections.
    pub fn start_incoming<S>(mut self, stream: S, secure: bool) -> SyncAddress<Self>
        where S: Stream<Item=(T, A), Error=io::Error> + 'static
    {
        if !self.sockets.is_empty() {
            let addrs: Vec<(net::SocketAddr, net::TcpListener)> =
                self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), &self.host, false);
            let workers = self.start_workers(&settings, &StreamHandlerType::Normal);

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting http server on {}", addr);
                self.accept.push(
                    start_accept_thread(sock, addr, self.backlog, workers.clone()));
            }
        }

        // set server settings
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let settings = ServerSettings::new(Some(addr), &self.host, secure);
        let mut apps: Vec<_> = (*self.factory)().into_iter().map(|h| h.into_handler()).collect();
        for app in &mut apps {
            app.server_settings(settings.clone());
        }
        self.h = Some(Rc::new(WorkerSettings::new(apps, self.keep_alive)));

        // start server
        HttpServer::create(move |ctx| {
            ctx.add_stream(stream.map(
                move |(t, _)| IoStream{io: t, peer: None, http2: false}));
            self
        })
    }
}

#[cfg(feature="signal")]
/// Unix Signals support
/// Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and send `SystemExit(0)`
/// message to `System` actor.
impl<T, A, H, U> Handler<signal::Signal> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static,
{
    fn handle(&mut self, msg: signal::Signal, ctx: &mut Context<Self>)
              -> Response<Self, signal::Signal>
    {
        match msg.0 {
            signal::SignalType::Int => {
                info!("SIGINT received, exiting");
                self.exit = true;
                Handler::<StopServer>::handle(self, StopServer{graceful: false}, ctx);
            }
            signal::SignalType::Term => {
                info!("SIGTERM received, stopping");
                self.exit = true;
                Handler::<StopServer>::handle(self, StopServer{graceful: true}, ctx);
            }
            signal::SignalType::Quit => {
                info!("SIGQUIT received, exiting");
                self.exit = true;
                Handler::<StopServer>::handle(self, StopServer{graceful: false}, ctx);
            }
            _ => (),
        };
        Self::empty()
    }
}

#[derive(Message)]
struct IoStream<T> {
    io: T,
    peer: Option<net::SocketAddr>,
    http2: bool,
}

impl<T, A, H, U> StreamHandler<IoStream<T>, io::Error> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static {}

impl<T, A, H, U> Handler<IoStream<T>, io::Error> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static,
{
    fn error(&mut self, err: io::Error, _: &mut Context<Self>) {
        debug!("Error handling request: {}", err)
    }

    fn handle(&mut self, msg: IoStream<T>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T>>
    {
        Arbiter::handle().spawn(
            HttpChannel::new(Rc::clone(self.h.as_ref().unwrap()), msg.io, msg.peer, msg.http2));
        Self::empty()
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
#[derive(Message)]
pub struct StopServer {
    pub graceful: bool
}

impl<T, A, H, U> Handler<PauseServer> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static,
{
    fn handle(&mut self, _: PauseServer, _: &mut Context<Self>) -> Response<Self, PauseServer>
    {
        for item in &self.accept {
            let _ = item.1.send(Command::Pause);
            let _ = item.0.set_readiness(mio::Ready::readable());
        }
        Self::empty()
    }
}

impl<T, A, H, U> Handler<ResumeServer> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static,
{
    fn handle(&mut self, _: ResumeServer, _: &mut Context<Self>) -> Response<Self, ResumeServer>
    {
        for item in &self.accept {
            let _ = item.1.send(Command::Resume);
            let _ = item.0.set_readiness(mio::Ready::readable());
        }
        Self::empty()
    }
}

impl<T, A, H, U> Handler<StopServer> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static,
{
    fn handle(&mut self, _: StopServer, ctx: &mut Context<Self>) -> Response<Self, StopServer>
    {
        for item in &self.accept {
            let _ = item.1.send(Command::Stop);
            let _ = item.0.set_readiness(mio::Ready::readable());
        }
        ctx.stop();

        // we need to stop system if server was spawned
        if self.exit {
            Arbiter::system().send(msgs::SystemExit(0))
        }
        Self::empty()
    }
}

/// Http worker
///
/// Worker accepts Socket objects via unbounded channel and start requests processing.
struct Worker<H> {
    h: Rc<WorkerSettings<H>>,
    hnd: Handle,
    handler: StreamHandlerType,
}

pub(crate) struct WorkerSettings<H> {
    h: RefCell<Vec<H>>,
    enabled: bool,
    keep_alive: u64,
    bytes: Rc<helpers::SharedBytesPool>,
    messages: Rc<helpers::SharedMessagePool>,
}

impl<H> WorkerSettings<H> {
    pub(crate) fn new(h: Vec<H>, keep_alive: Option<u64>) -> WorkerSettings<H> {
        WorkerSettings {
            h: RefCell::new(h),
            enabled: if let Some(ka) = keep_alive { ka > 0 } else { false },
            keep_alive: keep_alive.unwrap_or(0),
            bytes: Rc::new(helpers::SharedBytesPool::new()),
            messages: Rc::new(helpers::SharedMessagePool::new()),
        }
    }

    pub fn handlers(&self) -> RefMut<Vec<H>> {
        self.h.borrow_mut()
    }
    pub fn keep_alive(&self) -> u64 {
        self.keep_alive
    }
    pub fn keep_alive_enabled(&self) -> bool {
        self.enabled
    }
    pub fn get_shared_bytes(&self) -> helpers::SharedBytes {
        helpers::SharedBytes::new(self.bytes.get_bytes(), Rc::clone(&self.bytes))
    }
    pub fn get_http_message(&self) -> helpers::SharedHttpMessage {
        helpers::SharedHttpMessage::new(self.messages.get(), Rc::clone(&self.messages))
    }
}

impl<H: 'static> Worker<H> {

    fn new(h: Vec<H>, handler: StreamHandlerType, keep_alive: Option<u64>) -> Worker<H> {
        Worker {
            h: Rc::new(WorkerSettings::new(h, keep_alive)),
            hnd: Arbiter::handle().clone(),
            handler: handler,
        }
    }
    
    fn update_time(&self, ctx: &mut Context<Self>) {
        helpers::update_date();
        ctx.run_later(Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }
}

impl<H: 'static> Actor for Worker<H> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<H> StreamHandler<IoStream<net::TcpStream>> for Worker<H>
    where H: HttpHandler + 'static {}

impl<H> Handler<IoStream<net::TcpStream>> for Worker<H>
    where H: HttpHandler + 'static,
{
    fn handle(&mut self, msg: IoStream<net::TcpStream>, _: &mut Context<Self>)
              -> Response<Self, IoStream<net::TcpStream>>
    {
        if !self.h.keep_alive_enabled() &&
            msg.io.set_keepalive(Some(Duration::new(75, 0))).is_err()
        {
            error!("Can not set socket keep-alive option");
        }
        self.handler.handle(Rc::clone(&self.h), &self.hnd, msg);
        Self::empty()
    }
}

#[derive(Clone)]
enum StreamHandlerType {
    Normal,
    #[cfg(feature="tls")]
    Tls(TlsAcceptor),
    #[cfg(feature="alpn")]
    Alpn(SslAcceptor),
}

impl StreamHandlerType {

    fn handle<H: HttpHandler>(&mut self,
                              h: Rc<WorkerSettings<H>>,
                              hnd: &Handle,
                              msg: IoStream<net::TcpStream>) {
        match *self {
            StreamHandlerType::Normal => {
                let io = TcpStream::from_stream(msg.io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(HttpChannel::new(h, io, msg.peer, msg.http2));
            }
            #[cfg(feature="tls")]
            StreamHandlerType::Tls(ref acceptor) => {
                let IoStream { io, peer, http2 } = msg;
                let io = TcpStream::from_stream(io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(
                    TlsAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => Arbiter::handle().spawn(
                                HttpChannel::new(h, io, peer, http2)),
                            Err(err) =>
                                trace!("Error during handling tls connection: {}", err),
                        };
                        future::result(Ok(()))
                    })
                );
            }
            #[cfg(feature="alpn")]
            StreamHandlerType::Alpn(ref acceptor) => {
                let IoStream { io, peer, .. } = msg;
                let io = TcpStream::from_stream(io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(
                    SslAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => {
                                let http2 = if let Some(p) = io.get_ref().ssl().selected_alpn_protocol()
                                {
                                    p.len() == 2 && &p == b"h2"
                                } else {
                                    false
                                };
                                Arbiter::handle().spawn(HttpChannel::new(h, io, peer, http2));
                            },
                            Err(err) =>
                                trace!("Error during handling tls connection: {}", err),
                        };
                        future::result(Ok(()))
                    })
                );
            }
        }
    }
}

enum Command {
    Pause,
    Resume,
    Stop,
}

fn start_accept_thread(sock: net::TcpListener, addr: net::SocketAddr, backlog: i32,
                       workers: Vec<mpsc::UnboundedSender<IoStream<net::TcpStream>>>)
                       -> (mio::SetReadiness, sync_mpsc::Sender<Command>)
{
    let (tx, rx) = sync_mpsc::channel();
    let (reg, readiness) = mio::Registration::new2();

    // start accept thread
    let _ = thread::Builder::new().name(format!("Accept on {}", addr)).spawn(move || {
        const SRV: mio::Token = mio::Token(0);
        const CMD: mio::Token = mio::Token(1);

        let mut server = Some(
            mio::net::TcpListener::from_listener(sock, &addr)
                .expect("Can not create mio::net::TcpListener"));

        // Create a poll instance
        let poll = match mio::Poll::new() {
            Ok(poll) => poll,
            Err(err) => panic!("Can not create mio::Poll: {}", err),
        };

        // Start listening for incoming connections
        if let Some(ref srv) = server {
            if let Err(err) = poll.register(
                srv, SRV, mio::Ready::readable(), mio::PollOpt::edge()) {
                panic!("Can not register io: {}", err);
            }
        }

        // Start listening for incommin commands
        if let Err(err) = poll.register(&reg, CMD,
                                        mio::Ready::readable(), mio::PollOpt::edge()) {
            panic!("Can not register Registration: {}", err);
        }

        // Create storage for events
        let mut events = mio::Events::with_capacity(128);

        let mut next = 0;
        loop {
            if let Err(err) = poll.poll(&mut events, None) {
                panic!("Poll error: {}", err);
            }

            for event in events.iter() {
                match event.token() {
                    SRV => {
                        if let Some(ref server) = server {
                            loop {
                                match server.accept_std() {
                                    Ok((sock, addr)) => {
                                        let msg = IoStream{
                                            io: sock, peer: Some(addr), http2: false};
                                        workers[next].unbounded_send(msg)
                                            .expect("worker thread died");
                                        next = (next + 1) % workers.len();
                                    },
                                    Err(err) => if err.kind() == io::ErrorKind::WouldBlock {
                                        break
                                    } else {
                                        error!("Error accepting connection: {:?}", err);
                                        return
                                    }
                                }
                            }
                        }
                    },
                    CMD => match rx.try_recv() {
                        Ok(cmd) => match cmd {
                            Command::Pause => if let Some(server) = server.take() {
                                if let Err(err) = poll.deregister(&server) {
                                    error!("Can not deregister server socket {}", err);
                                } else {
                                    info!("Paused accepting connections on {}", addr);
                                }
                            },
                            Command::Resume => {
                                let lst = create_tcp_listener(addr, backlog)
                                    .expect("Can not create net::TcpListener");

                                server = Some(
                                    mio::net::TcpListener::from_listener(lst, &addr)
                                        .expect("Can not create mio::net::TcpListener"));

                                if let Some(ref server) = server {
                                    if let Err(err) = poll.register(
                                        server, SRV, mio::Ready::readable(), mio::PollOpt::edge())
                                    {
                                        error!("Can not resume socket accept process: {}", err);
                                    } else {
                                        info!("Accepting connections on {} has been resumed",
                                              addr);
                                    }
                                }
                            },
                            Command::Stop => return,
                        },
                        Err(err) => match err {
                            sync_mpsc::TryRecvError::Empty => (),
                            sync_mpsc::TryRecvError::Disconnected => return,
                        }
                    },
                    _ => unreachable!(),
                }
            }
        }
    });

    (readiness, tx)
}

fn create_tcp_listener(addr: net::SocketAddr, backlog: i32) -> io::Result<net::TcpListener> {
    let builder = match addr {
        net::SocketAddr::V4(_) => TcpBuilder::new_v4()?,
        net::SocketAddr::V6(_) => TcpBuilder::new_v6()?,
    };
    builder.bind(addr)?;
    builder.reuse_address(true)?;
    Ok(builder.listen(backlog)?)
}

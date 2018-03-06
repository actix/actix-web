use std::{io, net, thread};
use std::rc::Rc;
use std::sync::{Arc, mpsc as sync_mpsc};
use std::time::Duration;
use std::collections::HashMap;

use actix::prelude::*;
use actix::actors::signal;
use futures::{Future, Sink, Stream};
use futures::sync::mpsc;
use tokio_io::{AsyncRead, AsyncWrite};
use mio;
use num_cpus;
use net2::TcpBuilder;

#[cfg(feature="tls")]
use native_tls::TlsAcceptor;

#[cfg(feature="alpn")]
use openssl::ssl::{AlpnError, SslAcceptorBuilder};

use helpers;
use super::{IntoHttpHandler, IoStream};
use super::{PauseServer, ResumeServer, StopServer};
use super::channel::{HttpChannel, WrapperStream};
use super::worker::{Conn, Worker, StreamHandlerType, StopWorker};
use super::settings::{ServerSettings, WorkerSettings};


/// An HTTP Server
pub struct HttpServer<H> where H: IntoHttpHandler + 'static
{
    h: Option<Rc<WorkerSettings<H::Handler>>>,
    threads: usize,
    backlog: i32,
    host: Option<String>,
    keep_alive: Option<u64>,
    factory: Arc<Fn() -> Vec<H> + Send + Sync>,
    workers: Vec<Addr<Syn, Worker<H::Handler>>>,
    sockets: HashMap<net::SocketAddr, net::TcpListener>,
    accept: Vec<(mio::SetReadiness, sync_mpsc::Sender<Command>)>,
    exit: bool,
    shutdown_timeout: u16,
    signals: Option<Addr<Syn, signal::ProcessSignals>>,
    no_signals: bool,
}

unsafe impl<H> Sync for HttpServer<H> where H: IntoHttpHandler {}
unsafe impl<H> Send for HttpServer<H> where H: IntoHttpHandler {}


impl<H> Actor for HttpServer<H> where H: IntoHttpHandler
{
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<H> HttpServer<H> where H: IntoHttpHandler + 'static
{
    /// Create new http server with application factory
    pub fn new<F, U>(factory: F) -> Self
        where F: Fn() -> U + Sync + Send + 'static,
              U: IntoIterator<Item=H> + 'static,
    {
        let f = move || {
            (factory)().into_iter().collect()
        };
        
        HttpServer{ h: None,
                    threads: num_cpus::get(),
                    backlog: 2048,
                    host: None,
                    keep_alive: None,
                    factory: Arc::new(f),
                    workers: Vec::new(),
                    sockets: HashMap::new(),
                    accept: Vec::new(),
                    exit: false,
                    shutdown_timeout: 30,
                    signals: None,
                    no_signals: false,
        }
    }

    fn update_time(&self, ctx: &mut Context<Self>) {
        helpers::update_date();
        ctx.run_later(Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
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

    /// Send `SystemExit` message to actix system
    ///
    /// `SystemExit` message stops currently running system arbiter and all
    /// nested arbiters.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    /// Set alternative address for `ProcessSignals` actor.
    pub fn signals(mut self, addr: Addr<Syn, signal::ProcessSignals>) -> Self {
        self.signals = Some(addr);
        self
    }

    /// Disable signal handling
    pub fn disable_signals(mut self) -> Self {
        self.no_signals = true;
        self
    }

    /// Timeout for graceful workers shutdown.
    ///
    /// After receiving a stop signal, workers have this much time to finish serving requests.
    /// Workers still alive after the timeout are force dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, sec: u16) -> Self {
        self.shutdown_timeout = sec;
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
                     -> Vec<mpsc::UnboundedSender<Conn<net::TcpStream>>>
    {
        // start workers
        let mut workers = Vec::new();
        for _ in 0..self.threads {
            let s = settings.clone();
            let (tx, rx) = mpsc::unbounded::<Conn<net::TcpStream>>();

            let h = handler.clone();
            let ka = self.keep_alive;
            let factory = Arc::clone(&self.factory);
            let addr = Arbiter::start(move |ctx: &mut Context<_>| {
                let apps: Vec<_> = (*factory)()
                    .into_iter()
                    .map(|h| h.into_handler(s.clone())).collect();
                ctx.add_message_stream(rx);
                Worker::new(apps, h, ka)
            });
            workers.push(tx);
            self.workers.push(addr);
        }
        info!("Starting {} http workers", self.threads);
        workers
    }

    // subscribe to os signals
    fn subscribe_to_signals(&self) -> Option<Addr<Syn, signal::ProcessSignals>> {
        if !self.no_signals {
            if let Some(ref signals) = self.signals {
                Some(signals.clone())
            } else {
                Some(Arbiter::system_registry().get::<signal::ProcessSignals>())
            }
        } else {
            None
        }
    }
}

impl<H: IntoHttpHandler> HttpServer<H>
{
    /// Start listening for incoming connections.
    ///
    /// This method starts number of http handler workers in separate threads.
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
    ///              .resource("/", |r| r.h(httpcodes::HttpOk)))
    ///         .bind("127.0.0.1:0").expect("Can not bind to 127.0.0.1:0")
    ///         .start();
    /// #  actix::Arbiter::system().do_send(actix::msgs::SystemExit(0));
    ///
    ///    let _ = sys.run();  // <- Run actix system, this method actually starts all async processes
    /// }
    /// ```
    pub fn start(mut self) -> Addr<Syn, Self>
    {
        if self.sockets.is_empty() {
            panic!("HttpServer::bind() has to be called before start()");
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
            let signals = self.subscribe_to_signals();
            let addr: Addr<Syn, _> = Actor::start(self);
            signals.map(|signals| signals.do_send(
                signal::Subscribe(addr.clone().recipient())));
            addr
        }
    }

    /// Spawn new thread and start listening for incoming connections.
    ///
    /// This method spawns new thread and starts new actix system. Other than that it is
    /// similar to `start()` method. This method blocks.
    ///
    /// This methods panics if no socket addresses get bound.
    ///
    /// ```rust,ignore
    /// # extern crate futures;
    /// # extern crate actix;
    /// # extern crate actix_web;
    /// # use futures::Future;
    /// use actix_web::*;
    ///
    /// fn main() {
    ///     HttpServer::new(
    ///         || Application::new()
    ///              .resource("/", |r| r.h(httpcodes::HttpOk)))
    ///         .bind("127.0.0.1:0").expect("Can not bind to 127.0.0.1:0")
    ///         .run();
    /// }
    /// ```
    pub fn run(mut self) {
        self.exit = true;
        self.no_signals = false;

        let _ = thread::spawn(move || {
            let sys = System::new("http-server");
            self.start();
            let _ = sys.run();
        }).join();
    }
}

#[cfg(feature="tls")]
impl<H: IntoHttpHandler> HttpServer<H>
{
    /// Start listening for incoming tls connections.
    pub fn start_tls(mut self, acceptor: TlsAcceptor) -> io::Result<Addr<Syn, Self>> {
        if self.sockets.is_empty() {
            Err(io::Error::new(io::ErrorKind::Other, "No socket addresses are bound"))
        } else {
            let addrs: Vec<(net::SocketAddr, net::TcpListener)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), &self.host, false);
            let workers = self.start_workers(&settings, &StreamHandlerType::Tls(acceptor));

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting tls http server on {}", addr);
                self.accept.push(
                    start_accept_thread(sock, addr, self.backlog, workers.clone()));
            }

            // start http server actor
            let signals = self.subscribe_to_signals();
            let addr: Addr<Syn, _> = Actor::start(self);
            signals.map(|signals| signals.do_send(
                signal::Subscribe(addr.clone().recipient())));
            Ok(addr)
        }
    }
}

#[cfg(feature="alpn")]
impl<H: IntoHttpHandler> HttpServer<H>
{
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn start_ssl(mut self, mut builder: SslAcceptorBuilder) -> io::Result<Addr<Syn, Self>>
    {
        if self.sockets.is_empty() {
            Err(io::Error::new(io::ErrorKind::Other, "No socket addresses are bound"))
        } else {
            // alpn support
            builder.set_alpn_protos(b"\x02h2\x08http/1.1")?;
            builder.set_alpn_select_callback(|_, protos| {
                const H2: &[u8] = b"\x02h2";
                if protos.windows(3).any(|window| window == H2) {
                    Ok(b"h2")
                } else {
                    Err(AlpnError::NOACK)
                }
            });

            let acceptor = builder.build();
            let addrs: Vec<(net::SocketAddr, net::TcpListener)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), &self.host, false);
            let workers = self.start_workers(&settings, &StreamHandlerType::Alpn(acceptor));

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting tls http server on {}", addr);
                self.accept.push(
                    start_accept_thread(sock, addr, self.backlog, workers.clone()));
            }

            // start http server actor
            let signals = self.subscribe_to_signals();
            let addr: Addr<Syn, _> = Actor::start(self);
            signals.map(|signals| signals.do_send(
                signal::Subscribe(addr.clone().recipient())));
            Ok(addr)
        }
    }
}

impl<H: IntoHttpHandler> HttpServer<H>
{
    /// Start listening for incoming connections from a stream.
    ///
    /// This method uses only one thread for handling incoming connections.
    pub fn start_incoming<T, A, S>(mut self, stream: S, secure: bool) -> Addr<Syn, Self>
        where S: Stream<Item=(T, A), Error=io::Error> + 'static,
              T: AsyncRead + AsyncWrite + 'static,
              A: 'static
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
        let apps: Vec<_> = (*self.factory)()
            .into_iter().map(|h| h.into_handler(settings.clone())).collect();
        self.h = Some(Rc::new(WorkerSettings::new(apps, self.keep_alive)));

        // start server
        let signals = self.subscribe_to_signals();
        let addr: Addr<Syn, _> = HttpServer::create(move |ctx| {
            ctx.add_message_stream(
                stream
                    .map_err(|_| ())
                    .map(move |(t, _)| Conn{io: WrapperStream::new(t), peer: None, http2: false}));
            self
        });
        signals.map(|signals| signals.do_send(
            signal::Subscribe(addr.clone().recipient())));
        addr
    }
}

/// Signals support
/// Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and send `SystemExit(0)`
/// message to `System` actor.
impl<H: IntoHttpHandler> Handler<signal::Signal> for HttpServer<H>
{
    type Result = ();

    fn handle(&mut self, msg: signal::Signal, ctx: &mut Context<Self>) {
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
        }
    }
}

impl<T, H> Handler<Conn<T>> for HttpServer<H>
    where T: IoStream,
          H: IntoHttpHandler,
{
    type Result = ();

    fn handle(&mut self, msg: Conn<T>, _: &mut Context<Self>) -> Self::Result {
        Arbiter::handle().spawn(
            HttpChannel::new(
                Rc::clone(self.h.as_ref().unwrap()), msg.io, msg.peer, msg.http2));
    }
}

impl<H: IntoHttpHandler> Handler<PauseServer> for HttpServer<H>
{
    type Result = ();

    fn handle(&mut self, _: PauseServer, _: &mut Context<Self>)
    {
        for item in &self.accept {
            let _ = item.1.send(Command::Pause);
            let _ = item.0.set_readiness(mio::Ready::readable());
        }
    }
}

impl<H: IntoHttpHandler> Handler<ResumeServer> for HttpServer<H>
{
    type Result = ();

    fn handle(&mut self, _: ResumeServer, _: &mut Context<Self>) {
        for item in &self.accept {
            let _ = item.1.send(Command::Resume);
            let _ = item.0.set_readiness(mio::Ready::readable());
        }
    }
}

impl<H: IntoHttpHandler> Handler<StopServer> for HttpServer<H>
{
    type Result = actix::Response<(), ()>;

    fn handle(&mut self, msg: StopServer, ctx: &mut Context<Self>) -> Self::Result {
        // stop accept threads
        for item in &self.accept {
            let _ = item.1.send(Command::Stop);
            let _ = item.0.set_readiness(mio::Ready::readable());
        }

        // stop workers
        let (tx, rx) = mpsc::channel(1);

        let dur = if msg.graceful {
            Some(Duration::new(u64::from(self.shutdown_timeout), 0))
        } else {
            None
        };
        for worker in &self.workers {
            let tx2 = tx.clone();
            let fut = worker.send(StopWorker{graceful: dur}).into_actor(self);
            ActorFuture::then(fut, move |_, slf, _| {
                slf.workers.pop();
                if slf.workers.is_empty() {
                    let _ = tx2.send(());

                    // we need to stop system if server was spawned
                    if slf.exit {
                        Arbiter::system().do_send(actix::msgs::SystemExit(0))
                    }
                }
                actix::fut::ok(())
            }).spawn(ctx);
        }

        if !self.workers.is_empty() {
            Response::async(
                rx.into_future().map(|_| ()).map_err(|_| ()))
        } else {
            // we need to stop system if server was spawned
            if self.exit {
                Arbiter::system().do_send(actix::msgs::SystemExit(0))
            }
            Response::reply(Ok(()))
        }
    }
}

enum Command {
    Pause,
    Resume,
    Stop,
}

fn start_accept_thread(sock: net::TcpListener, addr: net::SocketAddr, backlog: i32,
                       workers: Vec<mpsc::UnboundedSender<Conn<net::TcpStream>>>)
                       -> (mio::SetReadiness, sync_mpsc::Sender<Command>)
{
    let (tx, rx) = sync_mpsc::channel();
    let (reg, readiness) = mio::Registration::new2();

    // start accept thread
    let _ = thread::Builder::new().name(format!("Accept on {}", addr)).spawn(move || {
        const SRV: mio::Token = mio::Token(0);
        const CMD: mio::Token = mio::Token(1);

        let mut server = Some(
            mio::net::TcpListener::from_std(sock)
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

        // Start listening for incoming commands
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
                                        let msg = Conn{
                                            io: sock, peer: Some(addr), http2: false};
                                        workers[next].unbounded_send(msg)
                                            .expect("worker thread died");
                                        next = (next + 1) % workers.len();
                                    },
                                    Err(err) => {
                                        if err.kind() != io::ErrorKind::WouldBlock {
                                            error!("Error accepting connection: {:?}", err);
                                        }
                                        break
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
                                    mio::net::TcpListener::from_std(lst)
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
                            Command::Stop => {
                                if let Some(server) = server.take() {
                                    let _ = poll.deregister(&server);
                                }
                                return
                            },
                        },
                        Err(err) => match err {
                            sync_mpsc::TryRecvError::Empty => (),
                            sync_mpsc::TryRecvError::Disconnected => {
                                if let Some(server) = server.take() {
                                    let _ = poll.deregister(&server);
                                }
                                return
                            },
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
    builder.reuse_address(true)?;
    builder.bind(addr)?;
    Ok(builder.listen(backlog)?)
}

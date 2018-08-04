use std::rc::Rc;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Duration;
use std::{io, net};

use actix::{
    fut, signal, Actor, ActorFuture, Addr, Arbiter, AsyncContext, Context, Handler,
    Response, StreamHandler, System, WrapFuture,
};

use futures::sync::mpsc;
use futures::{Future, Sink, Stream};
use num_cpus;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tcp::TcpStream;

#[cfg(feature = "tls")]
use native_tls::TlsAcceptor;

#[cfg(feature = "alpn")]
use openssl::ssl::SslAcceptorBuilder;

#[cfg(feature = "rust-tls")]
use rustls::ServerConfig;

use super::accept::{AcceptLoop, AcceptNotify, Command};
use super::channel::{HttpChannel, WrapperStream};
use super::settings::{ServerSettings, WorkerSettings};
use super::worker::{Conn, StopWorker, Token, Worker, WorkerClient, WorkerFactory};
use super::{AcceptorService, IntoHttpHandler, IoStream, KeepAlive};
use super::{PauseServer, ResumeServer, StopServer};

/// An HTTP Server
pub struct HttpServer<H>
where
    H: IntoHttpHandler + 'static,
{
    threads: usize,
    factory: WorkerFactory<H>,
    workers: Vec<(usize, Addr<Worker>)>,
    accept: AcceptLoop,
    exit: bool,
    shutdown_timeout: u16,
    signals: Option<Addr<signal::ProcessSignals>>,
    no_http2: bool,
    no_signals: bool,
    settings: Option<Rc<WorkerSettings<H::Handler>>>,
}

pub(crate) enum ServerCommand {
    WorkerDied(usize),
}

impl<H> Actor for HttpServer<H>
where
    H: IntoHttpHandler,
{
    type Context = Context<Self>;
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
            factory: WorkerFactory::new(f),
            workers: Vec::new(),
            accept: AcceptLoop::new(),
            exit: false,
            shutdown_timeout: 30,
            signals: None,
            no_http2: false,
            no_signals: false,
            settings: None,
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
        self.factory.backlog = num;
        self
    }

    /// Sets the maximum per-worker number of concurrent connections.
    ///
    /// All socket listeners will stop accepting connections when this limit is reached
    /// for each worker.
    ///
    /// By default max connections is set to a 100k.
    pub fn maxconn(mut self, num: usize) -> Self {
        self.accept.maxconn(num);
        self
    }

    /// Sets the maximum per-worker concurrent connection establish process.
    ///
    /// All listeners will stop accepting connections when this limit is reached. It
    /// can be used to limit the global SSL CPU usage.
    ///
    /// By default max connections is set to a 256.
    pub fn maxconnrate(mut self, num: usize) -> Self {
        self.accept.maxconnrate(num);
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a `Os`.
    pub fn keep_alive<T: Into<KeepAlive>>(mut self, val: T) -> Self {
        self.factory.keep_alive = val.into();
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    pub fn server_hostname(mut self, val: String) -> Self {
        self.factory.host = Some(val);
        self
    }

    /// Stop actix system.
    ///
    /// `SystemExit` message stops currently running system.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    /// Set alternative address for `ProcessSignals` actor.
    pub fn signals(mut self, addr: Addr<signal::ProcessSignals>) -> Self {
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
        self.factory.addrs()
    }

    /// Get addresses of bound sockets and the scheme for it.
    ///
    /// This is useful when the server is bound from different sources
    /// with some sockets listening on http and some listening on https
    /// and the user should be presented with an enumeration of which
    /// socket requires which protocol.
    pub fn addrs_with_scheme(&self) -> Vec<(net::SocketAddr, &str)> {
        self.factory.addrs_with_scheme()
    }

    /// Use listener for accepting incoming connection requests
    ///
    /// HttpServer does not change any configuration for TcpListener,
    /// it needs to be configured before passing it to listen() method.
    pub fn listen(mut self, lst: net::TcpListener) -> Self {
        self.factory.listen(lst);
        self
    }

    /// Use listener for accepting incoming connection requests
    pub fn listen_with<A>(
        mut self, lst: net::TcpListener, acceptor: A,
    ) -> io::Result<Self>
    where
        A: AcceptorService<TcpStream> + Send + 'static,
    {
        self.factory.listen_with(lst, acceptor);
        Ok(self)
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
    pub fn listen_tls(
        self, lst: net::TcpListener, acceptor: TlsAcceptor,
    ) -> io::Result<Self> {
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
        let flags = if !self.no_http2 {
            ServerFlags::HTTP1
        } else {
            ServerFlags::HTTP1 | ServerFlags::HTTP2
        };

        self.listen_with(lst, OpensslAcceptor::with_flags(builder, flags)?)
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
    pub fn listen_rustls(
        self, lst: net::TcpListener, builder: ServerConfig,
    ) -> io::Result<Self> {
        use super::{RustlsAcceptor, ServerFlags};

        // alpn support
        let flags = if !self.no_http2 {
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
        self.factory.bind(addr)?;
        Ok(self)
    }

    /// Start listening for incoming connections with supplied acceptor.
    #[cfg_attr(feature = "cargo-clippy", allow(needless_pass_by_value))]
    pub fn bind_with<S, A>(mut self, addr: S, acceptor: A) -> io::Result<Self>
    where
        S: net::ToSocketAddrs,
        A: AcceptorService<TcpStream> + Send + 'static,
    {
        self.factory.bind_with(addr, &acceptor)?;
        Ok(self)
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

    fn start_workers(&mut self, notify: &AcceptNotify) -> Vec<WorkerClient> {
        // start workers
        let mut workers = Vec::new();
        for idx in 0..self.threads {
            let (worker, addr) = self.factory.start(idx, notify.clone());
            workers.push(worker);
            self.workers.push((idx, addr));
        }
        info!("Starting {} http workers", self.threads);
        workers
    }

    // subscribe to os signals
    fn subscribe_to_signals(&self) -> Option<Addr<signal::ProcessSignals>> {
        if !self.no_signals {
            if let Some(ref signals) = self.signals {
                Some(signals.clone())
            } else {
                Some(System::current().registry().get::<signal::ProcessSignals>())
            }
        } else {
            None
        }
    }
}

impl<H: IntoHttpHandler> HttpServer<H> {
    /// Start listening for incoming connections.
    ///
    /// This method starts number of http handler workers in separate threads.
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
    pub fn start(mut self) -> Addr<Self> {
        let sockets = self.factory.take_sockets();
        if sockets.is_empty() {
            panic!("HttpServer::bind() has to be called before start()");
        } else {
            let notify = self.accept.get_notify();
            let workers = self.start_workers(&notify);

            // start accept thread
            for sock in &sockets {
                info!("Starting server on http://{}", sock.addr);
            }
            let rx = self.accept.start(sockets, workers.clone());

            // start http server actor
            let signals = self.subscribe_to_signals();
            let addr = Actor::create(move |ctx| {
                ctx.add_stream(rx);
                self
            });
            if let Some(signals) = signals {
                signals.do_send(signal::Subscribe(addr.clone().recipient()))
            }
            addr
        }
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
    pub fn start_incoming<T, S>(mut self, stream: S, secure: bool) -> Addr<Self>
    where
        S: Stream<Item = T, Error = io::Error> + Send + 'static,
        T: AsyncRead + AsyncWrite + Send + 'static,
    {
        // set server settings
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let settings = ServerSettings::new(Some(addr), &self.factory.host, secure);
        let apps: Vec<_> = (*self.factory.factory)()
            .into_iter()
            .map(|h| h.into_handler())
            .collect();
        self.settings = Some(Rc::new(WorkerSettings::new(
            apps,
            self.factory.keep_alive,
            settings,
            AcceptNotify::default(),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        )));

        // start server
        let signals = self.subscribe_to_signals();
        let addr = HttpServer::create(move |ctx| {
            ctx.add_message_stream(stream.map_err(|_| ()).map(move |t| Conn {
                io: WrapperStream::new(t),
                token: Token::new(0),
                peer: None,
            }));
            self
        });

        if let Some(signals) = signals {
            signals.do_send(signal::Subscribe(addr.clone().recipient()))
        }
        addr
    }
}

/// Signals support
/// Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and stop actix system
/// message to `System` actor.
impl<H: IntoHttpHandler> Handler<signal::Signal> for HttpServer<H> {
    type Result = ();

    fn handle(&mut self, msg: signal::Signal, ctx: &mut Context<Self>) {
        match msg.0 {
            signal::SignalType::Int => {
                info!("SIGINT received, exiting");
                self.exit = true;
                Handler::<StopServer>::handle(self, StopServer { graceful: false }, ctx);
            }
            signal::SignalType::Term => {
                info!("SIGTERM received, stopping");
                self.exit = true;
                Handler::<StopServer>::handle(self, StopServer { graceful: true }, ctx);
            }
            signal::SignalType::Quit => {
                info!("SIGQUIT received, exiting");
                self.exit = true;
                Handler::<StopServer>::handle(self, StopServer { graceful: false }, ctx);
            }
            _ => (),
        }
    }
}

/// Commands from accept threads
impl<H: IntoHttpHandler> StreamHandler<ServerCommand, ()> for HttpServer<H> {
    fn finished(&mut self, _: &mut Context<Self>) {}

    fn handle(&mut self, msg: ServerCommand, _: &mut Context<Self>) {
        match msg {
            ServerCommand::WorkerDied(idx) => {
                let mut found = false;
                for i in 0..self.workers.len() {
                    if self.workers[i].0 == idx {
                        self.workers.swap_remove(i);
                        found = true;
                        break;
                    }
                }

                if found {
                    error!("Worker has died {:?}, restarting", idx);

                    let mut new_idx = self.workers.len();
                    'found: loop {
                        for i in 0..self.workers.len() {
                            if self.workers[i].0 == new_idx {
                                new_idx += 1;
                                continue 'found;
                            }
                        }
                        break;
                    }

                    let (worker, addr) =
                        self.factory.start(new_idx, self.accept.get_notify());
                    self.workers.push((new_idx, addr));
                    self.accept.send(Command::Worker(worker));
                }
            }
        }
    }
}

impl<T, H> Handler<Conn<T>> for HttpServer<H>
where
    T: IoStream,
    H: IntoHttpHandler,
{
    type Result = ();

    fn handle(&mut self, msg: Conn<T>, _: &mut Context<Self>) -> Self::Result {
        Arbiter::spawn(HttpChannel::new(
            Rc::clone(self.settings.as_ref().unwrap()),
            msg.io,
            msg.peer,
        ));
    }
}

impl<H: IntoHttpHandler> Handler<PauseServer> for HttpServer<H> {
    type Result = ();

    fn handle(&mut self, _: PauseServer, _: &mut Context<Self>) {
        self.accept.send(Command::Pause);
    }
}

impl<H: IntoHttpHandler> Handler<ResumeServer> for HttpServer<H> {
    type Result = ();

    fn handle(&mut self, _: ResumeServer, _: &mut Context<Self>) {
        self.accept.send(Command::Resume);
    }
}

impl<H: IntoHttpHandler> Handler<StopServer> for HttpServer<H> {
    type Result = Response<(), ()>;

    fn handle(&mut self, msg: StopServer, ctx: &mut Context<Self>) -> Self::Result {
        // stop accept threads
        self.accept.send(Command::Stop);

        // stop workers
        let (tx, rx) = mpsc::channel(1);

        let dur = if msg.graceful {
            Some(Duration::new(u64::from(self.shutdown_timeout), 0))
        } else {
            None
        };
        for worker in &self.workers {
            let tx2 = tx.clone();
            ctx.spawn(
                worker
                    .1
                    .send(StopWorker { graceful: dur })
                    .into_actor(self)
                    .then(move |_, slf, ctx| {
                        slf.workers.pop();
                        if slf.workers.is_empty() {
                            let _ = tx2.send(());

                            // we need to stop system if server was spawned
                            if slf.exit {
                                ctx.run_later(Duration::from_millis(300), |_, _| {
                                    System::current().stop();
                                });
                            }
                        }
                        fut::ok(())
                    }),
            );
        }

        if !self.workers.is_empty() {
            Response::async(rx.into_future().map(|_| ()).map_err(|_| ()))
        } else {
            // we need to stop system if server was spawned
            if self.exit {
                ctx.run_later(Duration::from_millis(300), |_, _| {
                    System::current().stop();
                });
            }
            Response::reply(Ok(()))
        }
    }
}

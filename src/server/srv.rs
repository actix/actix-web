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
use net2::TcpBuilder;
use num_cpus;
use tokio_io::{AsyncRead, AsyncWrite};

#[cfg(feature = "tls")]
use native_tls::TlsAcceptor;

#[cfg(feature = "alpn")]
use openssl::ssl::{AlpnError, SslAcceptorBuilder};

#[cfg(feature = "rust-tls")]
use rustls::ServerConfig;

use super::accept::{AcceptLoop, AcceptNotify, Command};
use super::channel::{HttpChannel, WrapperStream};
use super::settings::{ServerSettings, WorkerSettings};
use super::worker::{
    Conn, StopWorker, StreamHandlerType, Worker, WorkerClient, WorkersPool,
};
use super::{IntoHttpHandler, IoStream, KeepAlive};
use super::{PauseServer, ResumeServer, StopServer};

#[cfg(feature = "alpn")]
fn configure_alpn(builder: &mut SslAcceptorBuilder) -> io::Result<()> {
    builder.set_alpn_protos(b"\x02h2\x08http/1.1")?;
    builder.set_alpn_select_callback(|_, protos| {
        const H2: &[u8] = b"\x02h2";
        if protos.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else {
            Err(AlpnError::NOACK)
        }
    });
    Ok(())
}

/// An HTTP Server
pub struct HttpServer<H>
where
    H: IntoHttpHandler + 'static,
{
    h: Option<Rc<WorkerSettings<H::Handler>>>,
    threads: usize,
    backlog: i32,
    sockets: Vec<Socket>,
    pool: WorkersPool<H>,
    workers: Vec<(usize, Addr<Worker<H::Handler>>)>,
    accept: AcceptLoop,
    exit: bool,
    shutdown_timeout: u16,
    signals: Option<Addr<signal::ProcessSignals>>,
    no_http2: bool,
    no_signals: bool,
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

pub(crate) struct Socket {
    pub lst: net::TcpListener,
    pub addr: net::SocketAddr,
    pub tp: StreamHandlerType,
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
            h: None,
            threads: num_cpus::get(),
            backlog: 2048,
            pool: WorkersPool::new(f),
            workers: Vec::new(),
            sockets: Vec::new(),
            accept: AcceptLoop::new(),
            exit: false,
            shutdown_timeout: 30,
            signals: None,
            no_http2: false,
            no_signals: false,
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
    pub fn max_connections(mut self, num: usize) -> Self {
        self.accept.max_connections(num);
        self
    }

    /// Sets the maximum concurrent per-worker number of SSL handshakes.
    ///
    /// All listeners will stop accepting connections when this limit is reached. It
    /// can be used to limit the global SSL CPU usage regardless of each worker
    /// capacity.
    ///
    /// By default max connections is set to a 256.
    pub fn max_sslrate(mut self, num: usize) -> Self {
        self.accept.max_sslrate(num);
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is set to a `Os`.
    pub fn keep_alive<T: Into<KeepAlive>>(mut self, val: T) -> Self {
        self.pool.keep_alive = val.into();
        self
    }

    /// Set server host name.
    ///
    /// Host name is used by application router aa a hostname for url
    /// generation. Check [ConnectionInfo](./dev/struct.ConnectionInfo.
    /// html#method.host) documentation for more information.
    pub fn server_hostname(mut self, val: String) -> Self {
        self.pool.host = Some(val);
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
        self.sockets.iter().map(|s| s.addr).collect()
    }

    /// Get addresses of bound sockets and the scheme for it.
    ///
    /// This is useful when the server is bound from different sources
    /// with some sockets listening on http and some listening on https
    /// and the user should be presented with an enumeration of which
    /// socket requires which protocol.
    pub fn addrs_with_scheme(&self) -> Vec<(net::SocketAddr, &str)> {
        self.sockets
            .iter()
            .map(|s| (s.addr, s.tp.scheme()))
            .collect()
    }

    /// Use listener for accepting incoming connection requests
    ///
    /// HttpServer does not change any configuration for TcpListener,
    /// it needs to be configured before passing it to listen() method.
    pub fn listen(mut self, lst: net::TcpListener) -> Self {
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            lst,
            tp: StreamHandlerType::Normal,
        });
        self
    }

    #[cfg(feature = "tls")]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// HttpServer does not change any configuration for TcpListener,
    /// it needs to be configured before passing it to listen() method.
    pub fn listen_tls(mut self, lst: net::TcpListener, acceptor: TlsAcceptor) -> Self {
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            lst,
            tp: StreamHandlerType::Tls(acceptor.clone()),
        });
        self
    }

    #[cfg(feature = "alpn")]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_ssl(
        mut self, lst: net::TcpListener, mut builder: SslAcceptorBuilder,
    ) -> io::Result<Self> {
        // alpn support
        if !self.no_http2 {
            configure_alpn(&mut builder)?;
        }
        let acceptor = builder.build();
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            lst,
            tp: StreamHandlerType::Alpn(acceptor.clone()),
        });
        Ok(self)
    }

    #[cfg(feature = "rust-tls")]
    /// Use listener for accepting incoming tls connection requests
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn listen_rustls(
        mut self, lst: net::TcpListener, mut builder: ServerConfig,
    ) -> io::Result<Self> {
        // alpn support
        if !self.no_http2 {
            builder.set_protocols(&vec!["h2".to_string(), "http/1.1".to_string()]);
        }
        let addr = lst.local_addr().unwrap();
        self.sockets.push(Socket {
            addr,
            lst,
            tp: StreamHandlerType::Rustls(Arc::new(builder)),
        });
        Ok(self)
    }

    fn bind2<S: net::ToSocketAddrs>(&mut self, addr: S) -> io::Result<Vec<Socket>> {
        let mut err = None;
        let mut succ = false;
        let mut sockets = Vec::new();
        for addr in addr.to_socket_addrs()? {
            match create_tcp_listener(addr, self.backlog) {
                Ok(lst) => {
                    succ = true;
                    let addr = lst.local_addr().unwrap();
                    sockets.push(Socket {
                        lst,
                        addr,
                        tp: StreamHandlerType::Normal,
                    });
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

    /// The socket address to bind
    ///
    /// To bind multiple addresses this method can be called multiple times.
    pub fn bind<S: net::ToSocketAddrs>(mut self, addr: S) -> io::Result<Self> {
        let sockets = self.bind2(addr)?;
        self.sockets.extend(sockets);
        Ok(self)
    }

    #[cfg(feature = "tls")]
    /// The ssl socket address to bind
    ///
    /// To bind multiple addresses this method can be called multiple times.
    pub fn bind_tls<S: net::ToSocketAddrs>(
        mut self, addr: S, acceptor: TlsAcceptor,
    ) -> io::Result<Self> {
        let sockets = self.bind2(addr)?;
        self.sockets.extend(sockets.into_iter().map(|mut s| {
            s.tp = StreamHandlerType::Tls(acceptor.clone());
            s
        }));
        Ok(self)
    }

    #[cfg(feature = "alpn")]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_ssl<S: net::ToSocketAddrs>(
        mut self, addr: S, mut builder: SslAcceptorBuilder,
    ) -> io::Result<Self> {
        // alpn support
        if !self.no_http2 {
            configure_alpn(&mut builder)?;
        }

        let acceptor = builder.build();
        let sockets = self.bind2(addr)?;
        self.sockets.extend(sockets.into_iter().map(|mut s| {
            s.tp = StreamHandlerType::Alpn(acceptor.clone());
            s
        }));
        Ok(self)
    }

    #[cfg(feature = "rust-tls")]
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn bind_rustls<S: net::ToSocketAddrs>(
        mut self, addr: S, mut builder: ServerConfig,
    ) -> io::Result<Self> {
        // alpn support
        if !self.no_http2 {
            builder.set_protocols(&vec!["h2".to_string(), "http/1.1".to_string()]);
        }

        let builder = Arc::new(builder);
        let sockets = self.bind2(addr)?;
        self.sockets.extend(sockets.into_iter().map(move |mut s| {
            s.tp = StreamHandlerType::Rustls(builder.clone());
            s
        }));
        Ok(self)
    }

    fn start_workers(&mut self, notify: &AcceptNotify) -> Vec<WorkerClient> {
        // start workers
        let mut workers = Vec::new();
        for idx in 0..self.threads {
            let (worker, addr) = self.pool.start(idx, notify.clone());
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
        if self.sockets.is_empty() {
            panic!("HttpServer::bind() has to be called before start()");
        } else {
            let mut addrs: Vec<(usize, Socket)> = Vec::new();

            for socket in self.sockets.drain(..) {
                let token = self.pool.insert(socket.addr, socket.tp.clone());
                addrs.push((token, socket));
            }
            let notify = self.accept.get_notify();
            let workers = self.start_workers(&notify);

            // start accept thread
            for (_, sock) in &addrs {
                info!("Starting server on http://{}", sock.addr);
            }
            let rx = self.accept.start(addrs, workers.clone());

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

#[doc(hidden)]
#[cfg(feature = "tls")]
#[deprecated(
    since = "0.6.0",
    note = "please use `actix_web::HttpServer::bind_tls` instead"
)]
impl<H: IntoHttpHandler> HttpServer<H> {
    /// Start listening for incoming tls connections.
    pub fn start_tls(mut self, acceptor: TlsAcceptor) -> io::Result<Addr<Self>> {
        for sock in &mut self.sockets {
            match sock.tp {
                StreamHandlerType::Normal => (),
                _ => continue,
            }
            sock.tp = StreamHandlerType::Tls(acceptor.clone());
        }
        Ok(self.start())
    }
}

#[doc(hidden)]
#[cfg(feature = "alpn")]
#[deprecated(
    since = "0.6.0",
    note = "please use `actix_web::HttpServer::bind_ssl` instead"
)]
impl<H: IntoHttpHandler> HttpServer<H> {
    /// Start listening for incoming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn start_ssl(
        mut self, mut builder: SslAcceptorBuilder,
    ) -> io::Result<Addr<Self>> {
        // alpn support
        if !self.no_http2 {
            builder.set_alpn_protos(b"\x02h2\x08http/1.1")?;
            builder.set_alpn_select_callback(|_, protos| {
                const H2: &[u8] = b"\x02h2";
                if protos.windows(3).any(|window| window == H2) {
                    Ok(b"h2")
                } else {
                    Err(AlpnError::NOACK)
                }
            });
        }

        let acceptor = builder.build();
        for sock in &mut self.sockets {
            match sock.tp {
                StreamHandlerType::Normal => (),
                _ => continue,
            }
            sock.tp = StreamHandlerType::Alpn(acceptor.clone());
        }
        Ok(self.start())
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
        let settings = ServerSettings::new(Some(addr), &self.pool.host, secure);
        let apps: Vec<_> = (*self.pool.factory)()
            .into_iter()
            .map(|h| h.into_handler())
            .collect();
        self.h = Some(Rc::new(WorkerSettings::new(
            apps,
            self.pool.keep_alive,
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
                token: 0,
                peer: None,
                http2: false,
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
                        self.pool.start(new_idx, self.accept.get_notify());
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
            Rc::clone(self.h.as_ref().unwrap()),
            msg.io,
            msg.peer,
            msg.http2,
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

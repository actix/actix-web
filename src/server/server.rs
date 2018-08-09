use std::{mem, net};
use std::time::Duration;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use num_cpus;
use futures::{Future, Stream, Sink};
use futures::sync::{mpsc, mpsc::unbounded};

use actix::{fut, signal, Actor, ActorFuture, Addr, Arbiter, AsyncContext,
            Context, Handler, Response, System, StreamHandler, WrapFuture};

use super::accept::{AcceptLoop, AcceptNotify, Command};
use super::worker::{StopWorker, Worker, WorkerClient, Conn};
use super::{PauseServer, ResumeServer, StopServer, Token};

pub trait Service: Send + 'static {
    /// Clone service
    fn clone(&self) -> Box<Service>;

    /// Create service handler for this service
    fn create(&self, conn: Connections) -> Box<ServiceHandler>;
}

impl Service for Box<Service> {
    fn clone(&self) -> Box<Service> {
        self.as_ref().clone()
    }

    fn create(&self, conn: Connections) -> Box<ServiceHandler> {
        self.as_ref().create(conn)
    }
}

pub trait ServiceHandler {
    /// Handle incoming stream
    fn handle(&mut self, token: Token, io: net::TcpStream, peer: Option<net::SocketAddr>);

    /// Shutdown open handlers
    fn shutdown(&self, _: bool) {}
}

pub(crate) enum ServerCommand {
    WorkerDied(usize),
}

pub struct Server {
    threads: usize,
    workers: Vec<(usize, Addr<Worker>)>,
    services: Vec<Box<Service>>,
    sockets: Vec<Vec<(Token, net::TcpListener)>>,
    accept: AcceptLoop,
    exit: bool,
    shutdown_timeout: u16,
    signals: Option<Addr<signal::ProcessSignals>>,
    no_signals: bool,
    maxconn: usize,
    maxconnrate: usize,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// Create new Server instance
    pub fn new() -> Server {
        Server {
            threads: num_cpus::get(),
            workers: Vec::new(),
            services: Vec::new(),
            sockets: Vec::new(),
            accept: AcceptLoop::new(),
            exit: false,
            shutdown_timeout: 30,
            signals: None,
            no_signals: false,
            maxconn: 102_400,
            maxconnrate: 256,
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
        self.maxconnrate= num;
        self
    }

    /// Stop actix system.
    ///
    /// `SystemExit` message stops currently running system.
    pub fn system_exit(mut self) -> Self {
        self.exit = true;
        self
    }

    #[doc(hidden)]
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

    /// Add new service to server
    pub fn service<T>(mut self, srv: T) -> Self 
    where
        T: Into<(Box<Service>, Vec<(Token, net::TcpListener)>)>
    {
        let (srv, sockets) = srv.into();
        self.services.push(srv);
        self.sockets.push(sockets);
        self
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
    ///     Server::new().
    ///         .service( 
    ///            HttpServer::new(|| App::new().resource("/", |r| r.h(|_| HttpResponse::Ok())))
    ///                .bind("127.0.0.1:0")
    ///                .expect("Can not bind to 127.0.0.1:0"))
    ///         .run();
    /// }
    /// ```
    pub fn run(self) {
        let sys = System::new("http-server");
        self.start();
        sys.run();
    }

    /// Start
    pub fn start(mut self) -> Addr<Server> {
        if self.sockets.is_empty() {
            panic!("Service should have at least one bound socket");
        } else {
            info!("Starting {} http workers", self.threads);

            // start workers
            let mut workers = Vec::new();
            for idx in 0..self.threads {
                let (addr, worker) = self.start_worker(idx, self.accept.get_notify());
                workers.push(worker);
                self.workers.push((idx, addr));
            }

            // start accept thread
            for sock in &self.sockets {
                for s in sock.iter() {
                    info!("Starting server on http://{:?}", s.1.local_addr().ok());
                }
            }
            let rx = self.accept.start(
                mem::replace(&mut self.sockets, Vec::new()), workers);

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

    fn start_worker(&self, idx: usize, notify: AcceptNotify) -> (Addr<Worker>, WorkerClient) {
        let (tx, rx) = unbounded::<Conn<net::TcpStream>>();
        let conns = Connections::new(notify, self.maxconn, self.maxconnrate);
        let worker = WorkerClient::new(idx, tx, conns.clone());
        let services: Vec<_> = self.services.iter().map(|v| v.clone()).collect();

        let addr = Arbiter::start(move |ctx: &mut Context<_>| {
            ctx.add_message_stream(rx);
            let handlers: Vec<_> = services.into_iter().map(|s| s.create(conns.clone())).collect();
            Worker::new(conns, handlers)
        });

        (addr, worker)
    }
}

impl Actor for Server
{
    type Context = Context<Self>;
}

/// Signals support
/// Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and stop actix system
/// message to `System` actor.
impl Handler<signal::Signal> for Server {
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

impl Handler<PauseServer> for Server {
    type Result = ();

    fn handle(&mut self, _: PauseServer, _: &mut Context<Self>) {
        self.accept.send(Command::Pause);
    }
}

impl Handler<ResumeServer> for Server {
    type Result = ();

    fn handle(&mut self, _: ResumeServer, _: &mut Context<Self>) {
        self.accept.send(Command::Resume);
    }
}

impl Handler<StopServer> for Server {
    type Result = Response<(), ()>;

    fn handle(&mut self, msg: StopServer, ctx: &mut Context<Self>) -> Self::Result {
        // stop accept thread
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

/// Commands from accept threads
impl StreamHandler<ServerCommand, ()> for Server {
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

                    let (addr, worker) = self.start_worker(new_idx, self.accept.get_notify());
                    self.workers.push((new_idx, addr));
                    self.accept.send(Command::Worker(worker));
                }
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Connections (Arc<ConnectionsInner>);

impl Connections {
    fn new(notify: AcceptNotify, maxconn: usize, maxconnrate: usize) -> Self {
        let maxconn_low = if maxconn > 10 { maxconn - 10 } else { 0 };
        let maxconnrate_low = if maxconnrate > 10 {
            maxconnrate - 10
        } else {
            0
        };

        Connections (
            Arc::new(ConnectionsInner {
                notify,
                maxconn, maxconnrate,
                maxconn_low, maxconnrate_low,
                conn: AtomicUsize::new(0),
                connrate: AtomicUsize::new(0),
            }))
    }

    pub(crate) fn available(&self) -> bool {
        self.0.available()
    }

    pub(crate) fn num_connections(&self) -> usize {
        self.0.conn.load(Ordering::Relaxed)
    }

    /// Report opened connection
    pub fn connection(&self) -> ConnectionTag {
        ConnectionTag::new(self.0.clone())
    }

    /// Report rate connection, rate is usually ssl handshake
    pub fn connection_rate(&self) -> ConnectionRateTag {
        ConnectionRateTag::new(self.0.clone())
    }
}

#[derive(Default)]
struct ConnectionsInner {
    notify: AcceptNotify,
    conn: AtomicUsize,
    connrate: AtomicUsize,
    maxconn: usize,
    maxconnrate: usize,
    maxconn_low: usize,
    maxconnrate_low: usize,
}

impl ConnectionsInner {
    fn available(&self) -> bool {
        if self.maxconnrate <= self.connrate.load(Ordering::Relaxed) {
            false
        } else {
            self.maxconn > self.conn.load(Ordering::Relaxed)
        }
    }

    fn notify_maxconn(&self, maxconn: usize) {
        if maxconn > self.maxconn_low && maxconn <= self.maxconn {
            self.notify.notify();
        }
    }
    
    fn notify_maxconnrate(&self, connrate: usize) {
        if connrate > self.maxconnrate_low && connrate <= self.maxconnrate {
            self.notify.notify();
        }
    }

}

/// Type responsible for max connection stat.
/// 
/// Max connections stat get updated on drop. 
pub struct ConnectionTag(Arc<ConnectionsInner>);

impl ConnectionTag {
    fn new(inner: Arc<ConnectionsInner>) -> Self {
        inner.conn.fetch_add(1, Ordering::Relaxed);
        ConnectionTag(inner)
    }
}

impl Drop for ConnectionTag {
    fn drop(&mut self) {
        let conn = self.0.conn.fetch_sub(1, Ordering::Relaxed);
        self.0.notify_maxconn(conn);
    }
}

/// Type responsible for max connection rate stat.
/// 
/// Max connections rate stat get updated on drop. 
pub struct ConnectionRateTag (Arc<ConnectionsInner>);

impl ConnectionRateTag {
    fn new(inner: Arc<ConnectionsInner>) -> Self {
        inner.connrate.fetch_add(1, Ordering::Relaxed);
        ConnectionRateTag(inner)
    }
}

impl Drop for ConnectionRateTag {
    fn drop(&mut self) {
        let connrate = self.0.connrate.fetch_sub(1, Ordering::Relaxed);
        self.0.notify_maxconnrate(connrate);
    }
}

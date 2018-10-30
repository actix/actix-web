use std::time::Duration;
use std::{io, mem, net};

use futures::sync::{mpsc, mpsc::unbounded};
use futures::{Future, Sink, Stream};
use net2::TcpBuilder;
use num_cpus;

use actix::{
    actors::signal, fut, msgs::Execute, Actor, ActorFuture, Addr, Arbiter, AsyncContext,
    Context, Handler, Response, StreamHandler, System, WrapFuture,
};

use super::accept::{AcceptLoop, AcceptNotify, Command};
use super::services::{InternalServiceFactory, StreamNewService, StreamServiceFactory};
use super::services::{ServiceFactory, ServiceNewService};
use super::worker::{self, Worker, WorkerAvailability, WorkerClient};
use super::{PauseServer, ResumeServer, StopServer, Token};

pub(crate) enum ServerCommand {
    WorkerDied(usize),
}

/// Server
pub struct Server {
    threads: usize,
    workers: Vec<(usize, WorkerClient)>,
    services: Vec<Box<InternalServiceFactory>>,
    sockets: Vec<(Token, net::TcpListener)>,
    accept: AcceptLoop,
    exit: bool,
    shutdown_timeout: Duration,
    signals: Option<Addr<signal::ProcessSignals>>,
    no_signals: bool,
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
            shutdown_timeout: Duration::from_secs(30),
            signals: None,
            no_signals: false,
        }
    }

    /// Set number of workers to start.
    ///
    /// By default server uses number of available logical cpu as threads
    /// count.
    pub fn workers(mut self, num: usize) -> Self {
        self.threads = num;
        self
    }

    /// Sets the maximum per-worker number of concurrent connections.
    ///
    /// All socket listeners will stop accepting connections when this limit is
    /// reached for each worker.
    ///
    /// By default max connections is set to a 25k per worker.
    pub fn maxconn(self, num: usize) -> Self {
        worker::max_concurrent_connections(num);
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

    /// Timeout for graceful workers shutdown in seconds.
    ///
    /// After receiving a stop signal, workers have this much time to finish
    /// serving requests. Workers still alive after the timeout are force
    /// dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, sec: u16) -> Self {
        self.shutdown_timeout = Duration::from_secs(u64::from(sec));
        self
    }

    /// Run external configuration as part of the server building
    /// process
    ///
    /// This function is useful for moving parts of configuration to a
    /// different module or event library.
    pub fn configure<F>(self, cfg: F) -> Server
    where
        F: Fn(Server) -> Server,
    {
        cfg(self)
    }

    /// Add new service to server
    pub fn bind<F, U, N: AsRef<str>>(mut self, name: N, addr: U, factory: F) -> io::Result<Self>
    where
        F: StreamServiceFactory,
        U: net::ToSocketAddrs,
    {
        let sockets = bind_addr(addr)?;

        for lst in sockets {
            self = self.listen(name.as_ref(), lst, factory.clone())
        }
        Ok(self)
    }

    /// Add new service to server
    pub fn listen<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: net::TcpListener,
        factory: F,
    ) -> Self
    where
        F: StreamServiceFactory,
    {
        let token = Token(self.services.len());
        self.services
            .push(StreamNewService::create(name.as_ref().to_string(), factory));
        self.sockets.push((token, lst));
        self
    }

    /// Add new service to server
    pub fn listen2<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: net::TcpListener,
        factory: F,
    ) -> Self
    where
        F: ServiceFactory,
    {
        let token = Token(self.services.len());
        self.services.push(ServiceNewService::create(
            name.as_ref().to_string(),
            factory,
        ));
        self.sockets.push((token, lst));
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

    /// Starts Server Actor and returns its address
    pub fn start(mut self) -> Addr<Server> {
        if self.sockets.is_empty() {
            panic!("Service should have at least one bound socket");
        } else {
            info!("Starting {} workers", self.threads);

            // start workers
            let mut workers = Vec::new();
            for idx in 0..self.threads {
                let worker = self.start_worker(idx, self.accept.get_notify());
                workers.push(worker.clone());
                self.workers.push((idx, worker));
            }

            // start accept thread
            for sock in &self.sockets {
                info!("Starting server on {}", sock.1.local_addr().ok().unwrap());
            }
            let rx = self
                .accept
                .start(mem::replace(&mut self.sockets, Vec::new()), workers);

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

    fn start_worker(&self, idx: usize, notify: AcceptNotify) -> WorkerClient {
        let (tx, rx) = unbounded();
        let timeout = self.shutdown_timeout;
        let avail = WorkerAvailability::new(notify);
        let worker = WorkerClient::new(idx, tx, avail.clone());
        let services: Vec<Box<InternalServiceFactory>> =
            self.services.iter().map(|v| v.clone_factory()).collect();

        Arbiter::new(format!("actix-net-worker-{}", idx)).do_send(Execute::new(move || {
            Worker::start(rx, services, avail, timeout.clone());
            Ok::<_, ()>(())
        }));

        worker
    }
}

impl Actor for Server {
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

        for worker in &self.workers {
            let tx2 = tx.clone();
            ctx.spawn(
                worker
                    .1
                    .stop(msg.graceful)
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

                    let worker = self.start_worker(new_idx, self.accept.get_notify());
                    self.workers.push((new_idx, worker.clone()));
                    self.accept.send(Command::Worker(worker));
                }
            }
        }
    }
}

fn bind_addr<S: net::ToSocketAddrs>(addr: S) -> io::Result<Vec<net::TcpListener>> {
    let mut err = None;
    let mut succ = false;
    let mut sockets = Vec::new();
    for addr in addr.to_socket_addrs()? {
        match create_tcp_listener(addr) {
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

fn create_tcp_listener(addr: net::SocketAddr) -> io::Result<net::TcpListener> {
    let builder = match addr {
        net::SocketAddr::V4(_) => TcpBuilder::new_v4()?,
        net::SocketAddr::V6(_) => TcpBuilder::new_v6()?,
    };
    builder.reuse_address(true)?;
    builder.bind(addr)?;
    Ok(builder.listen(1024)?)
}

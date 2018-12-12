use std::time::Duration;
use std::{io, mem, net};

use actix_rt::{spawn, Arbiter, System};
use futures::future::{lazy, ok};
use futures::stream::futures_unordered;
use futures::sync::mpsc::{unbounded, UnboundedReceiver};
use futures::{Async, Future, Poll, Stream};
use log::{error, info};
use net2::TcpBuilder;
use num_cpus;
use tokio_timer::sleep;

use crate::accept::{AcceptLoop, AcceptNotify, Command};
use crate::config::{ConfiguredService, ServiceConfig};
use crate::server::{Server, ServerCommand};
use crate::services::{InternalServiceFactory, StreamNewService, StreamServiceFactory};
use crate::services::{ServiceFactory, ServiceNewService};
use crate::signals::{Signal, Signals};
use crate::worker::{self, Worker, WorkerAvailability, WorkerClient};
use crate::Token;

/// Server builder
pub struct ServerBuilder {
    threads: usize,
    token: Token,
    workers: Vec<(usize, WorkerClient)>,
    services: Vec<Box<InternalServiceFactory>>,
    sockets: Vec<(Token, net::TcpListener)>,
    accept: AcceptLoop,
    exit: bool,
    shutdown_timeout: Duration,
    no_signals: bool,
    cmd: UnboundedReceiver<ServerCommand>,
    server: Server,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerBuilder {
    /// Create new Server builder instance
    pub fn new() -> ServerBuilder {
        let (tx, rx) = unbounded();
        let server = Server::new(tx);

        ServerBuilder {
            threads: num_cpus::get(),
            token: Token(0),
            workers: Vec::new(),
            services: Vec::new(),
            sockets: Vec::new(),
            accept: AcceptLoop::new(server.clone()),
            exit: false,
            shutdown_timeout: Duration::from_secs(30),
            no_signals: false,
            cmd: rx,
            server,
        }
    }

    /// Set number of workers to start.
    ///
    /// By default server uses number of available logical cpu as workers
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

    /// Execute external configuration as part of the server building
    /// process.
    ///
    /// This function is useful for moving parts of configuration to a
    /// different module or even library.
    pub fn configure<F>(mut self, f: F) -> io::Result<ServerBuilder>
    where
        F: Fn(&mut ServiceConfig) -> io::Result<()>,
    {
        let mut cfg = ServiceConfig::new(self.threads);

        f(&mut cfg)?;

        if let Some(apply) = cfg.apply {
            let mut srv = ConfiguredService::new(apply);
            for (name, lst) in cfg.services {
                let token = self.token.next();
                srv.stream(token, name);
                self.sockets.push((token, lst));
            }
            self.services.push(Box::new(srv));
        }
        self.threads = cfg.threads;

        Ok(self)
    }

    /// Add new service to the server.
    pub fn bind<F, U, N: AsRef<str>>(mut self, name: N, addr: U, factory: F) -> io::Result<Self>
    where
        F: StreamServiceFactory,
        U: net::ToSocketAddrs,
    {
        let sockets = bind_addr(addr)?;

        let token = self.token.next();
        self.services.push(StreamNewService::create(
            name.as_ref().to_string(),
            token,
            factory,
        ));

        for lst in sockets {
            self.sockets.push((token, lst));
        }
        Ok(self)
    }

    /// Add new service to the server.
    pub fn listen<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: net::TcpListener,
        factory: F,
    ) -> Self
    where
        F: StreamServiceFactory,
    {
        let token = self.token.next();
        self.services.push(StreamNewService::create(
            name.as_ref().to_string(),
            token,
            factory,
        ));
        self.sockets.push((token, lst));
        self
    }

    /// Add new service to the server.
    pub fn listen2<F, N: AsRef<str>>(
        mut self,
        name: N,
        lst: net::TcpListener,
        factory: F,
    ) -> Self
    where
        F: ServiceFactory,
    {
        let token = self.token.next();
        self.services.push(ServiceNewService::create(
            name.as_ref().to_string(),
            token,
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

    /// Starts processing incoming connections and return server controller.
    pub fn start(mut self) -> Server {
        if self.sockets.is_empty() {
            panic!("Server should have at least one bound socket");
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
            self.accept
                .start(mem::replace(&mut self.sockets, Vec::new()), workers);

            // handle signals
            if !self.no_signals {
                Signals::start(self.server.clone());
            }

            // start http server actor
            let server = self.server.clone();
            spawn(self);
            server
        }
    }

    fn start_worker(&self, idx: usize, notify: AcceptNotify) -> WorkerClient {
        let (tx1, rx1) = unbounded();
        let (tx2, rx2) = unbounded();
        let timeout = self.shutdown_timeout;
        let avail = WorkerAvailability::new(notify);
        let worker = WorkerClient::new(idx, tx1, tx2, avail.clone());
        let services: Vec<Box<InternalServiceFactory>> =
            self.services.iter().map(|v| v.clone_factory()).collect();

        Arbiter::new().send(lazy(move || {
            Worker::start(rx1, rx2, services, avail, timeout);
            Ok::<_, ()>(())
        }));

        worker
    }

    fn handle_cmd(&mut self, item: ServerCommand) {
        match item {
            ServerCommand::Pause(tx) => {
                self.accept.send(Command::Pause);
                let _ = tx.send(());
            }
            ServerCommand::Resume(tx) => {
                self.accept.send(Command::Resume);
                let _ = tx.send(());
            }
            ServerCommand::Signal(sig) => {
                // Signals support
                // Handle `SIGINT`, `SIGTERM`, `SIGQUIT` signals and stop actix system
                match sig {
                    Signal::Int => {
                        info!("SIGINT received, exiting");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: false,
                            completion: None,
                        })
                    }
                    Signal::Term => {
                        info!("SIGTERM received, stopping");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: true,
                            completion: None,
                        })
                    }
                    Signal::Quit => {
                        info!("SIGQUIT received, exiting");
                        self.exit = true;
                        self.handle_cmd(ServerCommand::Stop {
                            graceful: false,
                            completion: None,
                        })
                    }
                    _ => (),
                }
            }
            ServerCommand::Stop {
                graceful,
                completion,
            } => {
                let exit = self.exit;

                // stop accept thread
                self.accept.send(Command::Stop);

                // stop workers
                if !self.workers.is_empty() {
                    spawn(
                        futures_unordered(
                            self.workers
                                .iter()
                                .map(move |worker| worker.1.stop(graceful)),
                        )
                        .collect()
                        .then(move |_| {
                            if let Some(tx) = completion {
                                let _ = tx.send(());
                            }
                            if exit {
                                spawn(sleep(Duration::from_millis(300)).then(|_| {
                                    System::current().stop();
                                    ok(())
                                }));
                            }
                            ok(())
                        }),
                    )
                } else {
                    // we need to stop system if server was spawned
                    if self.exit {
                        spawn(sleep(Duration::from_millis(300)).then(|_| {
                            System::current().stop();
                            ok(())
                        }));
                    }
                    if let Some(tx) = completion {
                        let _ = tx.send(());
                    }
                }
            }
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

impl Future for ServerBuilder {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.cmd.poll() {
                Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Ok(Async::Ready(Some(item))) => self.handle_cmd(item),
            }
        }
    }
}

pub(super) fn bind_addr<S: net::ToSocketAddrs>(addr: S) -> io::Result<Vec<net::TcpListener>> {
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

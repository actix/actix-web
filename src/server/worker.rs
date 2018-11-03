use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::{mem, net, time};

use futures::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::sync::oneshot;
use futures::{future, Async, Future, Poll, Stream};
use tokio_current_thread::spawn;
use tokio_timer::{sleep, Delay};

use actix::msgs::StopArbiter;
use actix::{Arbiter, Message};

use super::accept::AcceptNotify;
use super::services::{BoxedServerService, InternalServiceFactory, ServerMessage};
use super::Token;
use counter::Counter;

pub(crate) struct WorkerCommand(Conn);

/// Stop worker message. Returns `true` on successful shutdown
/// and `false` if some connections still alive.
pub(crate) struct StopCommand {
    graceful: bool,
    result: oneshot::Sender<bool>,
}

#[derive(Debug, Message)]
pub(crate) struct Conn {
    pub io: net::TcpStream,
    pub handler: Token,
    pub token: Token,
    pub peer: Option<net::SocketAddr>,
}

const MAX_CONNS: AtomicUsize = AtomicUsize::new(25600);

/// Sets the maximum per-worker number of concurrent connections.
///
/// All socket listeners will stop accepting connections when this limit is
/// reached for each worker.
///
/// By default max connections is set to a 25k per worker.
pub fn max_concurrent_connections(num: usize) {
    MAX_CONNS.store(num, Ordering::Relaxed);
}

pub(crate) fn num_connections() -> usize {
    MAX_CONNS_COUNTER.with(|conns| conns.total())
}

thread_local! {
    static MAX_CONNS_COUNTER: Counter =
        Counter::new(MAX_CONNS.load(Ordering::Relaxed));
}

#[derive(Clone)]
pub(crate) struct WorkerClient {
    pub idx: usize,
    tx1: UnboundedSender<WorkerCommand>,
    tx2: UnboundedSender<StopCommand>,
    avail: WorkerAvailability,
}

impl WorkerClient {
    pub fn new(
        idx: usize,
        tx1: UnboundedSender<WorkerCommand>,
        tx2: UnboundedSender<StopCommand>,
        avail: WorkerAvailability,
    ) -> Self {
        WorkerClient {
            idx,
            tx1,
            tx2,
            avail,
        }
    }

    pub fn send(&self, msg: Conn) -> Result<(), Conn> {
        self.tx1
            .unbounded_send(WorkerCommand(msg))
            .map_err(|msg| msg.into_inner().0)
    }

    pub fn available(&self) -> bool {
        self.avail.available()
    }

    pub fn stop(&self, graceful: bool) -> oneshot::Receiver<bool> {
        let (result, rx) = oneshot::channel();
        let _ = self.tx2.unbounded_send(StopCommand { graceful, result });
        rx
    }
}

#[derive(Clone)]
pub(crate) struct WorkerAvailability {
    notify: AcceptNotify,
    available: Arc<AtomicBool>,
}

impl WorkerAvailability {
    pub fn new(notify: AcceptNotify) -> Self {
        WorkerAvailability {
            notify,
            available: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    pub fn set(&self, val: bool) {
        let old = self.available.swap(val, Ordering::Release);
        if !old && val {
            self.notify.notify()
        }
    }
}

/// Service worker
///
/// Worker accepts Socket objects via unbounded channel and starts stream
/// processing.
pub(crate) struct Worker {
    rx: UnboundedReceiver<WorkerCommand>,
    rx2: UnboundedReceiver<StopCommand>,
    services: Vec<Option<(usize, BoxedServerService)>>,
    availability: WorkerAvailability,
    conns: Counter,
    factories: Vec<Box<InternalServiceFactory>>,
    state: WorkerState,
    shutdown_timeout: time::Duration,
}

impl Worker {
    pub(crate) fn start(
        rx: UnboundedReceiver<WorkerCommand>,
        rx2: UnboundedReceiver<StopCommand>,
        factories: Vec<Box<InternalServiceFactory>>,
        availability: WorkerAvailability,
        shutdown_timeout: time::Duration,
    ) {
        availability.set(false);
        let mut wrk = MAX_CONNS_COUNTER.with(|conns| Worker {
            rx,
            rx2,
            availability,
            factories,
            shutdown_timeout,
            services: Vec::new(),
            conns: conns.clone(),
            state: WorkerState::Unavailable(Vec::new()),
        });

        let mut fut = Vec::new();
        for (idx, factory) in wrk.factories.iter().enumerate() {
            fut.push(factory.create().map(move |res| {
                res.into_iter()
                    .map(|(t, s)| (idx, t, s))
                    .collect::<Vec<_>>()
            }));
        }
        spawn(
            future::join_all(fut)
                .map_err(|e| {
                    error!("Can not start worker: {:?}", e);
                    Arbiter::current().do_send(StopArbiter(0));
                }).and_then(move |services| {
                    for item in services {
                        for (idx, token, service) in item {
                            while token.0 >= wrk.services.len() {
                                wrk.services.push(None);
                            }
                            wrk.services[token.0] = Some((idx, service));
                        }
                    }
                    wrk
                }),
        );
    }

    fn shutdown(&mut self, force: bool) {
        if force {
            self.services.iter_mut().for_each(|h| {
                if let Some(h) = h {
                    let _ = h.1.call((None, ServerMessage::ForceShutdown));
                }
            });
        } else {
            let timeout = self.shutdown_timeout;
            self.services.iter_mut().for_each(move |h| {
                if let Some(h) = h {
                    let _ = h.1.call((None, ServerMessage::Shutdown(timeout.clone())));
                }
            });
        }
    }

    fn check_readiness(&mut self, trace: bool) -> Result<bool, (Token, usize)> {
        let mut ready = self.conns.available();
        let mut failed = None;
        for (token, service) in &mut self.services.iter_mut().enumerate() {
            if let Some(service) = service {
                match service.1.poll_ready() {
                    Ok(Async::Ready(_)) => {
                        if trace {
                            trace!(
                                "Service {:?} is available",
                                self.factories[service.0].name(Token(token))
                            );
                        }
                    }
                    Ok(Async::NotReady) => ready = false,
                    Err(_) => {
                        error!(
                            "Service {:?} readiness check returned error, restarting",
                            self.factories[service.0].name(Token(token))
                        );
                        failed = Some((Token(token), service.0));
                    }
                }
            }
        }
        if let Some(idx) = failed {
            Err(idx)
        } else {
            Ok(ready)
        }
    }
}

enum WorkerState {
    None,
    Available,
    Unavailable(Vec<Conn>),
    Restarting(
        usize,
        Token,
        Box<Future<Item = Vec<(Token, BoxedServerService)>, Error = ()>>,
    ),
    Shutdown(Delay, Delay, oneshot::Sender<bool>),
}

impl Future for Worker {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // `StopWorker` message handler
        match self.rx2.poll() {
            Ok(Async::Ready(Some(StopCommand { graceful, result }))) => {
                self.availability.set(false);
                let num = num_connections();
                if num == 0 {
                    info!("Shutting down worker, 0 connections");
                    let _ = result.send(true);
                    return Ok(Async::Ready(()));
                } else if graceful {
                    self.shutdown(false);
                    let num = num_connections();
                    if num != 0 {
                        info!("Graceful worker shutdown, {} connections", num);
                        self.state = WorkerState::Shutdown(
                            sleep(time::Duration::from_secs(1)),
                            sleep(self.shutdown_timeout),
                            result,
                        );
                    } else {
                        let _ = result.send(true);
                        return Ok(Async::Ready(()));
                    }
                } else {
                    info!("Force shutdown worker, {} connections", num);
                    self.shutdown(true);
                    let _ = result.send(false);
                    return Ok(Async::Ready(()));
                }
            }
            _ => (),
        }

        let state = mem::replace(&mut self.state, WorkerState::None);

        match state {
            WorkerState::Unavailable(mut conns) => {
                match self.check_readiness(true) {
                    Ok(true) => {
                        self.state = WorkerState::Available;

                        // process requests from wait queue
                        while let Some(msg) = conns.pop() {
                            match self.check_readiness(false) {
                                Ok(true) => {
                                    let guard = self.conns.get();
                                    let _ = self.services[msg.handler.0]
                                        .as_mut()
                                        .expect("actix net bug")
                                        .1
                                        .call((Some(guard), ServerMessage::Connect(msg.io)));
                                }
                                Ok(false) => {
                                    trace!("Worker is unavailable");
                                    self.state = WorkerState::Unavailable(conns);
                                    return self.poll();
                                }
                                Err((token, idx)) => {
                                    trace!(
                                        "Service {:?} failed, restarting",
                                        self.factories[idx].name(token)
                                    );
                                    self.state = WorkerState::Restarting(
                                        idx,
                                        token,
                                        self.factories[idx].create(),
                                    );
                                    return self.poll();
                                }
                            }
                        }
                        self.availability.set(true);
                        return self.poll();
                    }
                    Ok(false) => {
                        self.state = WorkerState::Unavailable(conns);
                        return Ok(Async::NotReady);
                    }
                    Err((token, idx)) => {
                        trace!(
                            "Service {:?} failed, restarting",
                            self.factories[idx].name(token)
                        );
                        self.state =
                            WorkerState::Restarting(idx, token, self.factories[idx].create());
                        return self.poll();
                    }
                }
            }
            WorkerState::Restarting(idx, token, mut fut) => {
                match fut.poll() {
                    Ok(Async::Ready(item)) => {
                        for (token, service) in item {
                            trace!(
                                "Service {:?} has been restarted",
                                self.factories[idx].name(token)
                            );
                            self.services[token.0] = Some((idx, service));
                            self.state = WorkerState::Unavailable(Vec::new());
                        }
                    }
                    Ok(Async::NotReady) => {
                        self.state = WorkerState::Restarting(idx, token, fut);
                        return Ok(Async::NotReady);
                    }
                    Err(_) => {
                        panic!(
                            "Can not restart {:?} service",
                            self.factories[idx].name(token)
                        );
                    }
                }
                return self.poll();
            }
            WorkerState::Shutdown(mut t1, mut t2, tx) => {
                let num = num_connections();
                if num == 0 {
                    let _ = tx.send(true);
                    Arbiter::current().do_send(StopArbiter(0));
                    return Ok(Async::Ready(()));
                }

                // check graceful timeout
                match t2.poll().unwrap() {
                    Async::NotReady => (),
                    Async::Ready(_) => {
                        self.shutdown(true);
                        let _ = tx.send(false);
                        Arbiter::current().do_send(StopArbiter(0));
                        return Ok(Async::Ready(()));
                    }
                }

                // sleep for 1 second and then check again
                match t1.poll().unwrap() {
                    Async::NotReady => (),
                    Async::Ready(_) => {
                        t1 = sleep(time::Duration::from_secs(1));
                        let _ = t1.poll();
                    }
                }
                self.state = WorkerState::Shutdown(t1, t2, tx);
                return Ok(Async::NotReady);
            }
            WorkerState::Available => {
                loop {
                    match self.rx.poll() {
                        // handle incoming tcp stream
                        Ok(Async::Ready(Some(WorkerCommand(msg)))) => {
                            match self.check_readiness(false) {
                                Ok(true) => {
                                    let guard = self.conns.get();
                                    let _ = self.services[msg.handler.0]
                                        .as_mut()
                                        .expect("actix net bug")
                                        .1
                                        .call((Some(guard), ServerMessage::Connect(msg.io)));
                                    continue;
                                }
                                Ok(false) => {
                                    trace!("Worker is unavailable");
                                    self.availability.set(false);
                                    self.state = WorkerState::Unavailable(vec![msg]);
                                }
                                Err((token, idx)) => {
                                    trace!(
                                        "Service {:?} failed, restarting",
                                        self.factories[idx].name(token)
                                    );
                                    self.availability.set(false);
                                    self.state = WorkerState::Restarting(
                                        idx,
                                        token,
                                        self.factories[idx].create(),
                                    );
                                }
                            }
                            return self.poll();
                        }
                        Ok(Async::NotReady) => {
                            self.state = WorkerState::Available;
                            return Ok(Async::NotReady);
                        }
                        Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                    }
                }
            }
            WorkerState::None => panic!(),
        };
    }
}

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::{mem, net, time};

use futures::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::sync::oneshot;
use futures::task::AtomicTask;
use futures::{future, Async, Future, Poll, Stream};
use tokio_current_thread::spawn;
use tokio_timer::{sleep, Delay};

use actix::msgs::StopArbiter;
use actix::{Arbiter, Message};

use super::accept::AcceptNotify;
use super::services::{BoxedServerService, InternalServerServiceFactory, ServerMessage};
use super::Token;

pub(crate) enum WorkerCommand {
    Message(Conn),
    /// Stop worker message. Returns `true` on successful shutdown
    /// and `false` if some connections still alive.
    Stop(Option<time::Duration>, oneshot::Sender<bool>),
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
    static MAX_CONNS_COUNTER: Connections =
        Connections::new(MAX_CONNS.load(Ordering::Relaxed));
}

#[derive(Clone)]
pub(crate) struct WorkerClient {
    pub idx: usize,
    tx: UnboundedSender<WorkerCommand>,
    avail: WorkerAvailability,
}

impl WorkerClient {
    pub fn new(
        idx: usize, tx: UnboundedSender<WorkerCommand>, avail: WorkerAvailability,
    ) -> Self {
        WorkerClient { idx, tx, avail }
    }

    pub fn send(&self, msg: Conn) -> Result<(), Conn> {
        self.tx
            .unbounded_send(WorkerCommand::Message(msg))
            .map_err(|e| match e.into_inner() {
                WorkerCommand::Message(msg) => msg,
                _ => panic!(),
            })
    }

    pub fn available(&self) -> bool {
        self.avail.available()
    }

    pub fn stop(&self, graceful: Option<time::Duration>) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.unbounded_send(WorkerCommand::Stop(graceful, tx));
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

/// Http worker
///
/// Worker accepts Socket objects via unbounded channel and start requests
/// processing.
pub(crate) struct Worker {
    rx: UnboundedReceiver<WorkerCommand>,
    services: Vec<BoxedServerService>,
    availability: WorkerAvailability,
    conns: Connections,
    factories: Vec<Box<InternalServerServiceFactory>>,
    state: WorkerState,
}

impl Worker {
    pub(crate) fn start(
        rx: UnboundedReceiver<WorkerCommand>,
        factories: Vec<Box<InternalServerServiceFactory>>, availability: WorkerAvailability,
    ) {
        availability.set(false);
        let mut wrk = MAX_CONNS_COUNTER.with(|conns| Worker {
            rx,
            availability,
            factories,
            services: Vec::new(),
            conns: conns.clone(),
            state: WorkerState::Unavailable(Vec::new()),
        });

        let mut fut = Vec::new();
        for factory in &wrk.factories {
            fut.push(factory.create());
        }
        spawn(
            future::join_all(fut)
                .map_err(|e| {
                    error!("Can not start worker: {:?}", e);
                    Arbiter::current().do_send(StopArbiter(0));
                }).and_then(move |services| {
                    wrk.services.extend(services);
                    wrk
                }),
        );
    }

    fn shutdown(&mut self, force: bool) {
        if force {
            self.services.iter_mut().for_each(|h| {
                h.call(ServerMessage::ForceShutdown);
            });
        } else {
            self.services.iter_mut().for_each(|h| {
                h.call(ServerMessage::Shutdown);
            });
        }
    }

    fn check_readiness(&mut self) -> Result<bool, usize> {
        let mut ready = self.conns.check();
        let mut failed = None;
        for (idx, service) in self.services.iter_mut().enumerate() {
            match service.poll_ready() {
                Ok(Async::Ready(_)) => (),
                Ok(Async::NotReady) => ready = false,
                Err(_) => {
                    error!("Service readiness check returned error, restarting");
                    failed = Some(idx);
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
    Restarting(usize, Box<Future<Item = BoxedServerService, Error = ()>>),
    Shutdown(Delay, Delay, oneshot::Sender<bool>),
}

impl Future for Worker {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let state = mem::replace(&mut self.state, WorkerState::None);

        match state {
            WorkerState::Unavailable(mut conns) => {
                match self.check_readiness() {
                    Ok(true) => {
                        self.state = WorkerState::Available;

                        // process requests from wait queue
                        while let Some(msg) = conns.pop() {
                            match self.check_readiness() {
                                Ok(true) => {
                                    let guard = self.conns.get();
                                    spawn(
                                        self.services[msg.handler.0]
                                            .call(ServerMessage::Connect(msg.io))
                                            .map(|val| {
                                                drop(guard);
                                                val
                                            }),
                                    )
                                }
                                Ok(false) => {
                                    self.state = WorkerState::Unavailable(conns);
                                    return self.poll();
                                }
                                Err(idx) => {
                                    self.state = WorkerState::Restarting(
                                        idx,
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
                    Err(idx) => {
                        self.state = WorkerState::Restarting(idx, self.factories[idx].create());
                        return self.poll();
                    }
                }
            }
            WorkerState::Restarting(idx, mut fut) => {
                match fut.poll() {
                    Ok(Async::Ready(service)) => {
                        trace!("Service has been restarted");
                        self.services[idx] = service;
                        self.state = WorkerState::Unavailable(Vec::new());
                    }
                    Ok(Async::NotReady) => {
                        self.state = WorkerState::Restarting(idx, fut);
                        return Ok(Async::NotReady);
                    }
                    Err(_) => {
                        panic!("Can not restart service");
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
                        Ok(Async::Ready(Some(WorkerCommand::Message(msg)))) => match self
                            .check_readiness()
                        {
                            Ok(true) => {
                                let guard = self.conns.get();
                                spawn(
                                    self.services[msg.handler.0]
                                        .call(ServerMessage::Connect(msg.io))
                                        .map(|val| {
                                            drop(guard);
                                            val
                                        }),
                                );
                            }
                            Ok(false) => {
                                self.availability.set(false);
                                self.state = WorkerState::Unavailable(vec![msg]);
                            }
                            Err(idx) => {
                                self.availability.set(false);
                                self.state =
                                    WorkerState::Restarting(idx, self.factories[idx].create());
                            }
                        },
                        // `StopWorker` message handler
                        Ok(Async::Ready(Some(WorkerCommand::Stop(graceful, tx)))) => {
                            self.availability.set(false);
                            let num = num_connections();
                            if num == 0 {
                                info!("Shutting down http worker, 0 connections");
                                let _ = tx.send(true);
                                return Ok(Async::Ready(()));
                            } else if let Some(dur) = graceful {
                                self.shutdown(false);
                                let num = num_connections();
                                if num != 0 {
                                    info!("Graceful http worker shutdown, {} connections", num);
                                    break Some(WorkerState::Shutdown(
                                        sleep(time::Duration::from_secs(1)),
                                        sleep(dur),
                                        tx,
                                    ));
                                } else {
                                    let _ = tx.send(true);
                                    return Ok(Async::Ready(()));
                                }
                            } else {
                                info!("Force shutdown http worker, {} connections", num);
                                self.shutdown(true);
                                let _ = tx.send(false);
                                return Ok(Async::Ready(()));
                            }
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

        Ok(Async::NotReady)
    }
}

#[derive(Clone)]
pub(crate) struct Connections(Rc<ConnectionsInner>);

struct ConnectionsInner {
    count: Cell<usize>,
    maxconn: usize,
    task: AtomicTask,
}

impl Connections {
    pub fn new(maxconn: usize) -> Self {
        Connections(Rc::new(ConnectionsInner {
            maxconn,
            count: Cell::new(0),
            task: AtomicTask::new(),
        }))
    }

    pub fn get(&self) -> ConnectionsGuard {
        ConnectionsGuard::new(self.0.clone())
    }

    pub fn check(&self) -> bool {
        self.0.check()
    }

    pub fn total(&self) -> usize {
        self.0.count.get()
    }
}

pub(crate) struct ConnectionsGuard(Rc<ConnectionsInner>);

impl ConnectionsGuard {
    fn new(inner: Rc<ConnectionsInner>) -> Self {
        inner.inc();
        ConnectionsGuard(inner)
    }
}

impl Drop for ConnectionsGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

impl ConnectionsInner {
    fn inc(&self) {
        let num = self.count.get() + 1;
        self.count.set(num);
        if num == self.maxconn {
            self.task.register();
        }
    }

    fn dec(&self) {
        let num = self.count.get();
        self.count.set(num - 1);
        if num == self.maxconn {
            self.task.notify();
        }
    }

    fn check(&self) -> bool {
        self.count.get() < self.maxconn
    }
}

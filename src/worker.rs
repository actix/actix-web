use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::{net, time};

use futures::sync::mpsc::{SendError, UnboundedSender};
use futures::sync::oneshot;
use futures::task::AtomicTask;
use futures::{future, Async, Future, Poll};
use tokio_current_thread::spawn;

use actix::msgs::StopArbiter;
use actix::{
    fut, Actor, ActorContext, ActorFuture, Arbiter, AsyncContext, Context, Handler, Message,
    Response, WrapFuture,
};

use super::accept::AcceptNotify;
use super::server_service::{BoxedServerService, ServerMessage, ServerServiceFactory};
use super::Token;

#[derive(Message)]
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
    tx: UnboundedSender<Conn>,
    avail: WorkerAvailability,
}

impl WorkerClient {
    pub fn new(idx: usize, tx: UnboundedSender<Conn>, avail: WorkerAvailability) -> Self {
        WorkerClient { idx, tx, avail }
    }

    pub fn send(&self, msg: Conn) -> Result<(), SendError<Conn>> {
        self.tx.unbounded_send(msg)
    }

    pub fn available(&self) -> bool {
        self.avail.available()
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

/// Stop worker message. Returns `true` on successful shutdown
/// and `false` if some connections still alive.
pub(crate) struct StopWorker {
    pub graceful: Option<time::Duration>,
}

impl Message for StopWorker {
    type Result = Result<bool, ()>;
}

/// Http worker
///
/// Worker accepts Socket objects via unbounded channel and start requests
/// processing.
pub(crate) struct Worker {
    services: Vec<BoxedServerService>,
    availability: WorkerAvailability,
    conns: Connections,
}

impl Actor for Worker {
    type Context = Context<Self>;
}

impl Worker {
    pub(crate) fn new(
        ctx: &mut Context<Self>, services: Vec<Box<ServerServiceFactory + Send>>,
        availability: WorkerAvailability,
    ) -> Self {
        let wrk = MAX_CONNS_COUNTER.with(|conns| Worker {
            availability,
            services: Vec::new(),
            conns: conns.clone(),
        });

        ctx.wait(
            future::join_all(services.into_iter().map(|s| s.create()))
                .into_actor(&wrk)
                .map_err(|e, _, ctx| {
                    error!("Can not start worker: {:?}", e);
                    Arbiter::current().do_send(StopArbiter(0));
                    ctx.stop();
                }).and_then(|services, act, ctx| {
                    act.services.extend(services);
                    act.availability.set(true);
                    ctx.spawn(CheckReadiness(true));
                    fut::ok(())
                }),
        );

        wrk
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

    fn shutdown_timeout(
        &mut self, ctx: &mut Context<Worker>, tx: oneshot::Sender<bool>, dur: time::Duration,
    ) {
        // sleep for 1 second and then check again
        ctx.run_later(time::Duration::new(1, 0), move |slf, ctx| {
            let num = num_connections();
            if num == 0 {
                let _ = tx.send(true);
                Arbiter::current().do_send(StopArbiter(0));
            } else if let Some(d) = dur.checked_sub(time::Duration::new(1, 0)) {
                slf.shutdown_timeout(ctx, tx, d);
            } else {
                info!("Force shutdown http worker, {} connections", num);
                slf.shutdown(true);
                let _ = tx.send(false);
                Arbiter::current().do_send(StopArbiter(0));
            }
        });
    }
}

impl Handler<Conn> for Worker {
    type Result = ();

    fn handle(&mut self, msg: Conn, _: &mut Context<Self>) {
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
}

/// `StopWorker` message handler
impl Handler<StopWorker> for Worker {
    type Result = Response<bool, ()>;

    fn handle(&mut self, msg: StopWorker, ctx: &mut Context<Self>) -> Self::Result {
        let num = num_connections();
        if num == 0 {
            info!("Shutting down http worker, 0 connections");
            Response::reply(Ok(true))
        } else if let Some(dur) = msg.graceful {
            self.shutdown(false);
            let (tx, rx) = oneshot::channel();
            let num = num_connections();
            if num != 0 {
                info!("Graceful http worker shutdown, {} connections", num);
                self.shutdown_timeout(ctx, tx, dur);
                Response::reply(Ok(true))
            } else {
                Response::async(rx.map_err(|_| ()))
            }
        } else {
            info!("Force shutdown http worker, {} connections", num);
            self.shutdown(true);
            Response::reply(Ok(false))
        }
    }
}

struct CheckReadiness(bool);

impl ActorFuture for CheckReadiness {
    type Item = ();
    type Error = ();
    type Actor = Worker;

    fn poll(&mut self, act: &mut Worker, _: &mut Context<Worker>) -> Poll<(), ()> {
        let mut val = act.conns.check();
        if val {
            for service in &mut act.services {
                if let Ok(Async::NotReady) = service.poll_ready() {
                    val = false;
                    break;
                }
            }
        }
        if self.0 != val {
            self.0 = val;
            act.availability.set(val);
        }
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

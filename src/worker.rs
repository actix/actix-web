use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{net, time};

use futures::sync::mpsc::{SendError, UnboundedSender};
use futures::sync::oneshot;
use futures::{future, Async, Future, Poll};

use actix::msgs::StopArbiter;
use actix::{
    fut, Actor, ActorContext, ActorFuture, Arbiter, AsyncContext, Context, Handler, Message,
    Response, WrapFuture,
};

use super::accept::AcceptNotify;
use super::server_service::{self, BoxedServerService, ServerMessage, ServerServiceFactory};
use super::Token;

#[derive(Message)]
pub(crate) struct Conn {
    pub io: net::TcpStream,
    pub handler: Token,
    pub token: Token,
    pub peer: Option<net::SocketAddr>,
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
}

impl Actor for Worker {
    type Context = Context<Self>;
}

impl Worker {
    pub(crate) fn new(
        ctx: &mut Context<Self>, services: Vec<Box<ServerServiceFactory + Send>>,
        availability: WorkerAvailability,
    ) -> Self {
        let wrk = Worker {
            availability,
            services: Vec::new(),
        };

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
            let num = server_service::num_connections();
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
        Arbiter::spawn(self.services[msg.handler.0].call(ServerMessage::Connect(msg.io)))
    }
}

/// `StopWorker` message handler
impl Handler<StopWorker> for Worker {
    type Result = Response<bool, ()>;

    fn handle(&mut self, msg: StopWorker, ctx: &mut Context<Self>) -> Self::Result {
        let num = server_service::num_connections();
        if num == 0 {
            info!("Shutting down http worker, 0 connections");
            Response::reply(Ok(true))
        } else if let Some(dur) = msg.graceful {
            self.shutdown(false);
            let (tx, rx) = oneshot::channel();
            let num = server_service::num_connections();
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
        let mut val = true;
        for service in &mut act.services {
            if let Ok(Async::NotReady) = service.poll_ready() {
                val = false;
                break;
            }
        }
        if self.0 != val {
            self.0 = val;
            act.availability.set(val);
        }
        Ok(Async::NotReady)
    }
}

use std::{net, time};

use futures::sync::mpsc::{SendError, UnboundedSender};
use futures::sync::oneshot;
use futures::{future, Future};

use actix::msgs::StopArbiter;
use actix::{
    fut, Actor, ActorContext, ActorFuture, Arbiter, AsyncContext, Context, Handler, Message,
    Response, WrapFuture,
};

use super::server_service::{self, BoxedServerService, ServerServiceFactory};
use super::{server::Connections, Token};

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
    conns: Connections,
}

impl WorkerClient {
    pub fn new(idx: usize, tx: UnboundedSender<Conn>, conns: Connections) -> Self {
        WorkerClient { idx, tx, conns }
    }

    pub fn send(&self, msg: Conn) -> Result<(), SendError<Conn>> {
        self.tx.unbounded_send(msg)
    }

    pub fn available(&self) -> bool {
        self.conns.available()
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
    // conns: Connections,
    services: Vec<BoxedServerService>,
    // counters: Vec<Arc<AtomicUsize>>,
}

impl Actor for Worker {
    type Context = Context<Self>;
}

impl Worker {
    pub(crate) fn new(
        ctx: &mut Context<Self>, services: Vec<Box<ServerServiceFactory + Send>>,
    ) -> Self {
        let wrk = Worker {
            services: Vec::new(),
            // counters: services.iter().map(|i| i.counter()).collect(),
        };

        ctx.wait(
            future::join_all(services.into_iter().map(|s| s.create()))
                .into_actor(&wrk)
                .map_err(|e, _, ctx| {
                    error!("Can not start worker: {:?}", e);
                    Arbiter::current().do_send(StopArbiter(0));
                    ctx.stop();
                }).and_then(|services, act, _| {
                    act.services.extend(services);
                    fut::ok(())
                }),
        );

        wrk
    }

    fn shutdown(&self, _force: bool) {
        // self.services.iter().for_each(|h| h.shutdown(force));
    }

    fn shutdown_timeout(
        &self, ctx: &mut Context<Worker>, tx: oneshot::Sender<bool>, dur: time::Duration,
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
        Arbiter::spawn(self.services[msg.handler.0].call(msg.io))
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

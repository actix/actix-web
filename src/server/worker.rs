use std::{net, time};

use futures::sync::mpsc::{SendError, UnboundedSender};
use futures::sync::oneshot;
use futures::Future;

use actix::msgs::StopArbiter;
use actix::{Actor, Arbiter, AsyncContext, Context, Handler, Message, Response};

use super::server::{Connections, ServiceHandler};
use super::Token;

#[derive(Message)]
pub(crate) struct Conn<T> {
    pub io: T,
    pub handler: Token,
    pub token: Token,
    pub peer: Option<net::SocketAddr>,
}

pub(crate) struct Socket {
    pub lst: net::TcpListener,
    pub addr: net::SocketAddr,
    pub token: Token,
}

#[derive(Clone)]
pub(crate) struct WorkerClient {
    pub idx: usize,
    tx: UnboundedSender<Conn<net::TcpStream>>,
    conns: Connections,
}

impl WorkerClient {
    pub fn new(
        idx: usize, tx: UnboundedSender<Conn<net::TcpStream>>, conns: Connections,
    ) -> Self {
        WorkerClient { idx, tx, conns }
    }

    pub fn send(
        &self, msg: Conn<net::TcpStream>,
    ) -> Result<(), SendError<Conn<net::TcpStream>>> {
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
    conns: Connections,
    handlers: Vec<Box<ServiceHandler>>,
}

impl Actor for Worker {
    type Context = Context<Self>;
}

impl Worker {
    pub(crate) fn new(conns: Connections, handlers: Vec<Box<ServiceHandler>>) -> Self {
        Worker { conns, handlers }
    }

    fn shutdown(&self, force: bool) {
        self.handlers.iter().for_each(|h| h.shutdown(force));
    }

    fn shutdown_timeout(
        &self, ctx: &mut Context<Worker>, tx: oneshot::Sender<bool>, dur: time::Duration,
    ) {
        // sleep for 1 second and then check again
        ctx.run_later(time::Duration::new(1, 0), move |slf, ctx| {
            let num = slf.conns.num_connections();
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

impl Handler<Conn<net::TcpStream>> for Worker {
    type Result = ();

    fn handle(&mut self, msg: Conn<net::TcpStream>, _: &mut Context<Self>) {
        self.handlers[msg.handler.0].handle(msg.token, msg.io, msg.peer)
    }
}

/// `StopWorker` message handler
impl Handler<StopWorker> for Worker {
    type Result = Response<bool, ()>;

    fn handle(&mut self, msg: StopWorker, ctx: &mut Context<Self>) -> Self::Result {
        let num = self.conns.num_connections();
        if num == 0 {
            info!("Shutting down http worker, 0 connections");
            Response::reply(Ok(true))
        } else if let Some(dur) = msg.graceful {
            self.shutdown(false);
            let (tx, rx) = oneshot::channel();
            let num = self.conns.num_connections();
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

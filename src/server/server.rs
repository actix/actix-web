use futures::sync::mpsc::UnboundedSender;
use futures::sync::oneshot;
use futures::Future;

use super::builder::ServerBuilder;

pub(crate) enum ServerCommand {
    WorkerDied(usize),
    Pause(oneshot::Sender<()>),
    Resume(oneshot::Sender<()>),
    /// Whether to try and shut down gracefully
    Stop {
        graceful: bool,
        completion: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct Server(UnboundedSender<ServerCommand>);

impl Server {
    pub(crate) fn new(tx: UnboundedSender<ServerCommand>) -> Self {
        Server(tx)
    }

    /// Start server building process
    pub fn build() -> ServerBuilder {
        ServerBuilder::default()
    }

    pub(crate) fn worker_died(&self, idx: usize) {
        let _ = self.0.unbounded_send(ServerCommand::WorkerDied(idx));
    }

    /// Pause accepting incoming connections
    ///
    /// If socket contains some pending connection, they might be dropped.
    /// All opened connection remains active.
    pub fn pause(&self) -> impl Future<Item = (), Error = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.unbounded_send(ServerCommand::Pause(tx));
        rx.map_err(|_| ())
    }

    /// Resume accepting incoming connections
    pub fn resume(&self) -> impl Future<Item = (), Error = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.unbounded_send(ServerCommand::Resume(tx));
        rx.map_err(|_| ())
    }

    /// Stop incoming connection processing, stop all workers and exit.
    ///
    /// If server starts with `spawn()` method, then spawned thread get terminated.
    pub fn stop(&self, graceful: bool) -> impl Future<Item = (), Error = ()> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.unbounded_send(ServerCommand::Stop {
            graceful,
            completion: tx,
        });
        rx.map_err(|_| ())
    }
}

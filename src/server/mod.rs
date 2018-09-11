//! General purpose networking server

use actix::Message;

mod accept;
mod server;
mod services;
mod worker;

pub use self::server::Server;
pub use self::services::ServerServiceFactory;

pub(crate) use self::worker::Connections;

/// Pause accepting incoming connections
///
/// If socket contains some pending connection, they might be dropped.
/// All opened connection remains active.
#[derive(Message)]
pub struct PauseServer;

/// Resume accepting incoming connections
#[derive(Message)]
pub struct ResumeServer;

/// Stop incoming connection processing, stop all workers and exit.
///
/// If server starts with `spawn()` method, then spawned thread get terminated.
pub struct StopServer {
    /// Whether to try and shut down gracefully
    pub graceful: bool,
}

impl Message for StopServer {
    type Result = Result<(), ()>;
}

/// Socket id token
#[derive(Clone, Copy)]
pub(crate) struct Token(usize);

//! Thread pool for blocking operations

use std::fmt;

use derive_more::Display;
use futures::sync::oneshot;
use futures::{Async, Future, Poll};
use parking_lot::Mutex;
use threadpool::ThreadPool;

use crate::ResponseError;

/// Env variable for default cpu pool size
const ENV_CPU_POOL_VAR: &str = "ACTIX_CPU_POOL";

lazy_static::lazy_static! {
    pub(crate) static ref DEFAULT_POOL: Mutex<ThreadPool> = {
        let default = match std::env::var(ENV_CPU_POOL_VAR) {
            Ok(val) => {
                if let Ok(val) = val.parse() {
                    val
                } else {
                    log::error!("Can not parse ACTIX_CPU_POOL value");
                    num_cpus::get() * 5
                }
            }
            Err(_) => num_cpus::get() * 5,
        };
        Mutex::new(
            threadpool::Builder::new()
                .thread_name("actix-web".to_owned())
                .num_threads(default)
                .build(),
        )
    };
}

thread_local! {
    static POOL: ThreadPool = {
        DEFAULT_POOL.lock().clone()
    };
}

/// Blocking operation execution error
#[derive(Debug, Display)]
pub enum BlockingError<E: fmt::Debug> {
    #[display(fmt = "{:?}", _0)]
    Error(E),
    #[display(fmt = "Thread pool is gone")]
    Canceled,
}

impl<E: fmt::Debug> ResponseError for BlockingError<E> {}

/// Execute blocking function on a thread pool, returns future that resolves
/// to result of the function execution.
pub fn run<F, I, E>(f: F) -> CpuFuture<I, E>
where
    F: FnOnce() -> Result<I, E> + Send + 'static,
    I: Send + 'static,
    E: Send + fmt::Debug + 'static,
{
    let (tx, rx) = oneshot::channel();
    POOL.with(|pool| {
        pool.execute(move || {
            if !tx.is_canceled() {
                let _ = tx.send(f());
            }
        })
    });

    CpuFuture { rx }
}

/// Blocking operation completion future. It resolves with results
/// of blocking function execution.
pub struct CpuFuture<I, E> {
    rx: oneshot::Receiver<Result<I, E>>,
}

impl<I, E: fmt::Debug> Future for CpuFuture<I, E> {
    type Item = I;
    type Error = BlockingError<E>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let res =
            futures::try_ready!(self.rx.poll().map_err(|_| BlockingError::Canceled));
        match res {
            Ok(val) => Ok(Async::Ready(val)),
            Err(err) => Err(BlockingError::Error(err)),
        }
    }
}

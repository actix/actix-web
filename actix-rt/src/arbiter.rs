use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fmt, thread};

use futures::sync::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::sync::oneshot::{channel, Sender};
use futures::{future, Async, Future, IntoFuture, Poll, Stream};
use tokio_current_thread::spawn;

use crate::builder::Builder;
use crate::system::System;

thread_local!(
    static ADDR: RefCell<Option<Arbiter>> = RefCell::new(None);
    static RUNNING: Cell<bool> = Cell::new(false);
    static Q: RefCell<Vec<Box<Future<Item = (), Error = ()>>>> = RefCell::new(Vec::new());
);

pub(crate) static COUNT: AtomicUsize = AtomicUsize::new(0);

pub(crate) enum ArbiterCommand {
    Stop,
    Execute(Box<Future<Item = (), Error = ()> + Send>),
}

impl fmt::Debug for ArbiterCommand {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ArbiterCommand::Stop => write!(f, "ArbiterCommand::Stop"),
            ArbiterCommand::Execute(_) => write!(f, "ArbiterCommand::Execute"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Arbiter(UnboundedSender<ArbiterCommand>);

impl Default for Arbiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Arbiter {
    pub(crate) fn new_system() -> Self {
        let (tx, rx) = unbounded();

        let arb = Arbiter(tx);
        ADDR.with(|cell| *cell.borrow_mut() = Some(arb.clone()));
        RUNNING.with(|cell| cell.set(false));
        Arbiter::spawn(ArbiterController { stop: None, rx });

        arb
    }

    /// Returns current arbiter's address
    pub fn current() -> Arbiter {
        ADDR.with(|cell| match *cell.borrow() {
            Some(ref addr) => addr.clone(),
            None => panic!("Arbiter is not running"),
        })
    }

    /// Stop arbiter
    pub fn stop(&self) {
        let _ = self.0.unbounded_send(ArbiterCommand::Stop);
    }

    /// Spawn new thread and run event loop in spawned thread.
    /// Returns address of newly created arbiter.
    pub fn new() -> Arbiter {
        let id = COUNT.fetch_add(1, Ordering::Relaxed);
        let name = format!("actix-rt:worker:{}", id);
        let sys = System::current();
        let (arb_tx, arb_rx) = unbounded();
        let arb_tx2 = arb_tx.clone();

        let _ = thread::Builder::new().name(name.clone()).spawn(move || {
            let mut rt = Builder::new().build_rt().expect("Can not create Runtime");
            let arb = Arbiter(arb_tx);

            let (stop, stop_rx) = channel();
            RUNNING.with(|cell| cell.set(true));

            System::set_current(sys);

            // start arbiter controller
            rt.spawn(ArbiterController {
                stop: Some(stop),
                rx: arb_rx,
            });
            ADDR.with(|cell| *cell.borrow_mut() = Some(arb.clone()));

            // register arbiter
            let _ = System::current()
                .sys()
                .unbounded_send(SystemCommand::RegisterArbiter(id, arb.clone()));

            // run loop
            let _ = match rt.block_on(stop_rx) {
                Ok(code) => code,
                Err(_) => 1,
            };

            // unregister arbiter
            let _ = System::current()
                .sys()
                .unbounded_send(SystemCommand::UnregisterArbiter(id));
        });

        Arbiter(arb_tx2)
    }

    pub(crate) fn run_system() {
        RUNNING.with(|cell| cell.set(true));
        Q.with(|cell| {
            let mut v = cell.borrow_mut();
            for fut in v.drain(..) {
                spawn(fut);
            }
        });
    }

    pub(crate) fn stop_system() {
        RUNNING.with(|cell| cell.set(false));
    }

    /// Spawn a future on the current thread.
    pub fn spawn<F>(future: F)
    where
        F: Future<Item = (), Error = ()> + 'static,
    {
        RUNNING.with(move |cell| {
            if cell.get() {
                spawn(Box::new(future));
            } else {
                Q.with(move |cell| cell.borrow_mut().push(Box::new(future)));
            }
        });
    }

    /// Executes a future on the current thread.
    pub fn spawn_fn<F, R>(f: F)
    where
        F: FnOnce() -> R + 'static,
        R: IntoFuture<Item = (), Error = ()> + 'static,
    {
        Arbiter::spawn(future::lazy(f))
    }

    /// Send a future on the arbiter's thread and spawn.
    pub fn send<F>(&self, future: F)
    where
        F: Future<Item = (), Error = ()> + Send + 'static,
    {
        let _ = self
            .0
            .unbounded_send(ArbiterCommand::Execute(Box::new(future)));
    }
}

struct ArbiterController {
    stop: Option<Sender<i32>>,
    rx: UnboundedReceiver<ArbiterCommand>,
}

impl Drop for ArbiterController {
    fn drop(&mut self) {
        if thread::panicking() {
            eprintln!("Panic in Arbiter thread, shutting down system.");
            if System::current().stop_on_panic() {
                System::current().stop_with_code(1)
            }
        }
    }
}

impl Future for ArbiterController {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.rx.poll() {
                Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                Ok(Async::Ready(Some(item))) => match item {
                    ArbiterCommand::Stop => {
                        if let Some(stop) = self.stop.take() {
                            let _ = stop.send(0);
                        };
                        return Ok(Async::Ready(()));
                    }
                    ArbiterCommand::Execute(fut) => {
                        spawn(fut);
                    }
                },
                Ok(Async::NotReady) => return Ok(Async::NotReady),
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum SystemCommand {
    Exit(i32),
    RegisterArbiter(usize, Arbiter),
    UnregisterArbiter(usize),
}

#[derive(Debug)]
pub(crate) struct SystemArbiter {
    stop: Option<Sender<i32>>,
    commands: UnboundedReceiver<SystemCommand>,
    arbiters: HashMap<usize, Arbiter>,
}

impl SystemArbiter {
    pub(crate) fn new(stop: Sender<i32>, commands: UnboundedReceiver<SystemCommand>) -> Self {
        SystemArbiter {
            commands,
            stop: Some(stop),
            arbiters: HashMap::new(),
        }
    }
}

impl Future for SystemArbiter {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.commands.poll() {
                Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                Ok(Async::Ready(Some(cmd))) => match cmd {
                    SystemCommand::Exit(code) => {
                        // stop arbiters
                        for arb in self.arbiters.values() {
                            arb.stop();
                        }
                        // stop event loop
                        if let Some(stop) = self.stop.take() {
                            let _ = stop.send(code);
                        }
                    }
                    SystemCommand::RegisterArbiter(name, hnd) => {
                        self.arbiters.insert(name, hnd);
                    }
                    SystemCommand::UnregisterArbiter(name) => {
                        self.arbiters.remove(&name);
                    }
                },
                Ok(Async::NotReady) => return Ok(Async::NotReady),
            }
        }
    }
}

// /// Execute function in arbiter's thread
// impl<I: Send, E: Send> Handler<Execute<I, E>> for SystemArbiter {
//     type Result = Result<I, E>;

//     fn handle(&mut self, msg: Execute<I, E>, _: &mut Context<Self>) -> Result<I, E> {
//         msg.exec()
//     }
// }

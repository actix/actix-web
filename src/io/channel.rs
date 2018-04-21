#![allow(dead_code, unused_imports)]
use std::cell::UnsafeCell;
use std::net::{SocketAddr, TcpStream};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::{io, thread};

use bytes::Bytes;
use futures::task::{current as current_task, Task};
use mio;

use super::IoStream;
use io::{Core, IoToken};
use server::h1decoder::{Decoder, DecoderError, Message};

pub(crate) enum TaskCommand {
    Stream(IoStream),
}

pub(crate) enum IoCommand {
    AddSource(TcpStream, Option<SocketAddr>),
    Bytes(IoToken, Bytes),
    Pause(IoToken),
    Drain(IoToken),
    Resume(IoToken),
    Done { token: IoToken, graceful: bool },
}

struct Shared {
    io: AtomicBool,
    io_tx: mpsc::Sender<IoCommand>,
    io_rx: mpsc::Receiver<IoCommand>,
    io_reg: mio::Registration,
    io_notify: mio::SetReadiness,

    task: AtomicBool,
    task_tx: mpsc::Sender<TaskCommand>,
    task_rx: mpsc::Receiver<TaskCommand>,
    task_notify: UnsafeCell<Task>,
}

pub(crate) struct IoChannel {
    shared: Arc<Shared>,
}

impl IoChannel {
    pub fn new() -> Self {
        let (reg, notify) = mio::Registration::new2();
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();

        let shared = Arc::new(Shared {
            io: AtomicBool::new(true),
            io_tx: tx1,
            io_rx: rx1,
            io_reg: reg,
            io_notify: notify,

            task: AtomicBool::new(true),
            task_tx: tx2,
            task_rx: rx2,
            task_notify: UnsafeCell::new(current_task()),
        });

        let ch = TaskChannel {
            shared: Arc::clone(&shared),
        };
        thread::spawn(move || {
            let core = Core::new(ch).unwrap();
            core.run()
        });

        IoChannel { shared }
    }

    pub fn notify(&self) {
        if !self.shared.io.load(Ordering::Relaxed) {
            let _ = self.shared
                .io_notify
                .set_readiness(mio::Ready::readable());
        }
    }

    pub fn set_notify(&self, task: Task) {
        unsafe { *self.shared.task_notify.get() = task };
        self.shared.task.store(false, Ordering::Relaxed);
    }

    pub fn add_source(&self, io: TcpStream, peer: Option<SocketAddr>, _http2: bool) {
        self.send(IoCommand::AddSource(io, peer));
        self.notify();
    }

    #[inline]
    pub fn start(&self) {
        self.shared.task.store(true, Ordering::Relaxed)
    }

    #[inline]
    pub fn end(&self) {
        self.shared.task.store(false, Ordering::Relaxed)
    }

    #[inline]
    pub fn send(&self, msg: IoCommand) {
        let _ = self.shared.io_tx.send(msg);
        self.notify();
    }

    #[inline]
    pub fn try_recv(&self) -> Result<TaskCommand, mpsc::TryRecvError> {
        self.shared.task_rx.try_recv()
    }
}

pub(crate) struct TaskChannel {
    shared: Arc<Shared>,
}

unsafe impl Send for TaskChannel {}

impl TaskChannel {
    #[inline]
    pub fn notify(&self) {
        if !self.shared.task.load(Ordering::Relaxed) {
            let task = unsafe { &mut *self.shared.task_notify.get() };
            task.notify();
        }
    }

    #[inline]
    pub fn send(&self, msg: TaskCommand) {
        let _ = self.shared.task_tx.send(msg);
    }

    #[inline]
    pub fn registration(&self) -> &mio::Registration {
        &self.shared.io_reg
    }

    #[inline]
    pub fn start(&self) {
        self.shared.io.store(true, Ordering::Relaxed)
    }

    #[inline]
    pub fn end(&self) {
        self.shared.io.store(false, Ordering::Relaxed)
    }

    #[inline]
    pub fn try_recv(&self) -> Result<IoCommand, mpsc::TryRecvError> {
        self.shared.io_rx.try_recv()
    }
}

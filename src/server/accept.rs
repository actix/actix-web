use std::sync::mpsc as sync_mpsc;
use std::time::{Duration, Instant};
use std::{io, net, thread};

use futures::{sync::mpsc, Future};
use mio;
use slab::Slab;
use tokio_timer::Delay;

use actix::{msgs::Execute, Arbiter, System};

use super::server::ServerCommand;
use super::worker::{Conn, WorkerClient};
use super::Token;

pub(crate) enum Command {
    Pause,
    Resume,
    Stop,
    Worker(WorkerClient),
}

struct ServerSocketInfo {
    addr: net::SocketAddr,
    token: Token,
    handler: Token,
    sock: mio::net::TcpListener,
    timeout: Option<Instant>,
}

#[derive(Clone)]
pub(crate) struct AcceptNotify(mio::SetReadiness);

impl AcceptNotify {
    pub(crate) fn new(ready: mio::SetReadiness) -> Self {
        AcceptNotify(ready)
    }

    pub(crate) fn notify(&self) {
        let _ = self.0.set_readiness(mio::Ready::readable());
    }
}

impl Default for AcceptNotify {
    fn default() -> Self {
        AcceptNotify::new(mio::Registration::new2().1)
    }
}

pub(crate) struct AcceptLoop {
    cmd_reg: Option<mio::Registration>,
    cmd_ready: mio::SetReadiness,
    notify_reg: Option<mio::Registration>,
    notify_ready: mio::SetReadiness,
    tx: sync_mpsc::Sender<Command>,
    rx: Option<sync_mpsc::Receiver<Command>>,
    srv: Option<(
        mpsc::UnboundedSender<ServerCommand>,
        mpsc::UnboundedReceiver<ServerCommand>,
    )>,
}

impl AcceptLoop {
    pub fn new() -> AcceptLoop {
        let (tx, rx) = sync_mpsc::channel();
        let (cmd_reg, cmd_ready) = mio::Registration::new2();
        let (notify_reg, notify_ready) = mio::Registration::new2();

        AcceptLoop {
            tx,
            cmd_ready,
            cmd_reg: Some(cmd_reg),
            notify_ready,
            notify_reg: Some(notify_reg),
            rx: Some(rx),
            srv: Some(mpsc::unbounded()),
        }
    }

    pub fn send(&self, msg: Command) {
        let _ = self.tx.send(msg);
        let _ = self.cmd_ready.set_readiness(mio::Ready::readable());
    }

    pub fn get_notify(&self) -> AcceptNotify {
        AcceptNotify::new(self.notify_ready.clone())
    }

    pub(crate) fn start(
        &mut self, socks: Vec<Vec<(Token, net::TcpListener)>>,
        workers: Vec<WorkerClient>,
    ) -> mpsc::UnboundedReceiver<ServerCommand> {
        let (tx, rx) = self.srv.take().expect("Can not re-use AcceptInfo");

        Accept::start(
            self.rx.take().expect("Can not re-use AcceptInfo"),
            self.cmd_reg.take().expect("Can not re-use AcceptInfo"),
            self.notify_reg.take().expect("Can not re-use AcceptInfo"),
            socks,
            tx,
            workers,
        );
        rx
    }
}

struct Accept {
    poll: mio::Poll,
    rx: sync_mpsc::Receiver<Command>,
    sockets: Slab<ServerSocketInfo>,
    workers: Vec<WorkerClient>,
    srv: mpsc::UnboundedSender<ServerCommand>,
    timer: (mio::Registration, mio::SetReadiness),
    next: usize,
    backpressure: bool,
}

const DELTA: usize = 100;
const CMD: mio::Token = mio::Token(0);
const TIMER: mio::Token = mio::Token(1);
const NOTIFY: mio::Token = mio::Token(2);

/// This function defines errors that are per-connection. Which basically
/// means that if we get this error from `accept()` system call it means
/// next connection might be ready to be accepted.
///
/// All other errors will incur a timeout before next `accept()` is performed.
/// The timeout is useful to handle resource exhaustion errors like ENFILE
/// and EMFILE. Otherwise, could enter into tight loop.
fn connection_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::ConnectionRefused
        || e.kind() == io::ErrorKind::ConnectionAborted
        || e.kind() == io::ErrorKind::ConnectionReset
}

impl Accept {
    #![cfg_attr(feature = "cargo-clippy", allow(too_many_arguments))]
    pub(crate) fn start(
        rx: sync_mpsc::Receiver<Command>, cmd_reg: mio::Registration,
        notify_reg: mio::Registration, socks: Vec<Vec<(Token, net::TcpListener)>>,
        srv: mpsc::UnboundedSender<ServerCommand>, workers: Vec<WorkerClient>,
    ) {
        let sys = System::current();

        // start accept thread
        let _ = thread::Builder::new()
            .name("actix-web accept loop".to_owned())
            .spawn(move || {
                System::set_current(sys);
                let mut accept = Accept::new(rx, socks, workers, srv);

                // Start listening for incoming commands
                if let Err(err) = accept.poll.register(
                    &cmd_reg,
                    CMD,
                    mio::Ready::readable(),
                    mio::PollOpt::edge(),
                ) {
                    panic!("Can not register Registration: {}", err);
                }

                // Start listening for notify updates
                if let Err(err) = accept.poll.register(
                    &notify_reg,
                    NOTIFY,
                    mio::Ready::readable(),
                    mio::PollOpt::edge(),
                ) {
                    panic!("Can not register Registration: {}", err);
                }

                accept.poll();
            });
    }

    fn new(
        rx: sync_mpsc::Receiver<Command>, socks: Vec<Vec<(Token, net::TcpListener)>>,
        workers: Vec<WorkerClient>, srv: mpsc::UnboundedSender<ServerCommand>,
    ) -> Accept {
        // Create a poll instance
        let poll = match mio::Poll::new() {
            Ok(poll) => poll,
            Err(err) => panic!("Can not create mio::Poll: {}", err),
        };

        // Start accept
        let mut sockets = Slab::new();
        for (idx, srv_socks) in socks.into_iter().enumerate() {
            for (hnd_token, lst) in srv_socks {
                let addr = lst.local_addr().unwrap();
                let server = mio::net::TcpListener::from_std(lst)
                    .expect("Can not create mio::net::TcpListener");

                let entry = sockets.vacant_entry();
                let token = entry.key();

                // Start listening for incoming connections
                if let Err(err) = poll.register(
                    &server,
                    mio::Token(token + DELTA),
                    mio::Ready::readable(),
                    mio::PollOpt::edge(),
                ) {
                    panic!("Can not register io: {}", err);
                }

                entry.insert(ServerSocketInfo {
                    addr,
                    token: hnd_token,
                    handler: Token(idx),
                    sock: server,
                    timeout: None,
                });
            }
        }

        // Timer
        let (tm, tmr) = mio::Registration::new2();
        if let Err(err) =
            poll.register(&tm, TIMER, mio::Ready::readable(), mio::PollOpt::edge())
        {
            panic!("Can not register Registration: {}", err);
        }

        Accept {
            poll,
            rx,
            sockets,
            workers,
            srv,
            next: 0,
            timer: (tm, tmr),
            backpressure: false,
        }
    }

    fn poll(&mut self) {
        // Create storage for events
        let mut events = mio::Events::with_capacity(128);

        loop {
            if let Err(err) = self.poll.poll(&mut events, None) {
                panic!("Poll error: {}", err);
            }

            for event in events.iter() {
                let token = event.token();
                match token {
                    CMD => if !self.process_cmd() {
                        return;
                    },
                    TIMER => self.process_timer(),
                    NOTIFY => self.backpressure(false),
                    _ => {
                        let token = usize::from(token);
                        if token < DELTA {
                            continue;
                        }
                        self.accept(token - DELTA);
                    }
                }
            }
        }
    }

    fn process_timer(&mut self) {
        let now = Instant::now();
        for (token, info) in self.sockets.iter_mut() {
            if let Some(inst) = info.timeout.take() {
                if now > inst {
                    if let Err(err) = self.poll.register(
                        &info.sock,
                        mio::Token(token + DELTA),
                        mio::Ready::readable(),
                        mio::PollOpt::edge(),
                    ) {
                        error!("Can not register server socket {}", err);
                    } else {
                        info!("Resume accepting connections on {}", info.addr);
                    }
                } else {
                    info.timeout = Some(inst);
                }
            }
        }
    }

    fn process_cmd(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(cmd) => match cmd {
                    Command::Pause => {
                        for (_, info) in self.sockets.iter_mut() {
                            if let Err(err) = self.poll.deregister(&info.sock) {
                                error!("Can not deregister server socket {}", err);
                            } else {
                                info!("Paused accepting connections on {}", info.addr);
                            }
                        }
                    }
                    Command::Resume => {
                        for (token, info) in self.sockets.iter() {
                            if let Err(err) = self.poll.register(
                                &info.sock,
                                mio::Token(token + DELTA),
                                mio::Ready::readable(),
                                mio::PollOpt::edge(),
                            ) {
                                error!("Can not resume socket accept process: {}", err);
                            } else {
                                info!(
                                    "Accepting connections on {} has been resumed",
                                    info.addr
                                );
                            }
                        }
                    }
                    Command::Stop => {
                        for (_, info) in self.sockets.iter() {
                            let _ = self.poll.deregister(&info.sock);
                        }
                        return false;
                    }
                    Command::Worker(worker) => {
                        self.backpressure(false);
                        self.workers.push(worker);
                    }
                },
                Err(err) => match err {
                    sync_mpsc::TryRecvError::Empty => break,
                    sync_mpsc::TryRecvError::Disconnected => {
                        for (_, info) in self.sockets.iter() {
                            let _ = self.poll.deregister(&info.sock);
                        }
                        return false;
                    }
                },
            }
        }
        true
    }

    fn backpressure(&mut self, on: bool) {
        if self.backpressure {
            if !on {
                self.backpressure = false;
                for (token, info) in self.sockets.iter() {
                    if let Err(err) = self.poll.register(
                        &info.sock,
                        mio::Token(token + DELTA),
                        mio::Ready::readable(),
                        mio::PollOpt::edge(),
                    ) {
                        error!("Can not resume socket accept process: {}", err);
                    } else {
                        info!("Accepting connections on {} has been resumed", info.addr);
                    }
                }
            }
        } else if on {
            self.backpressure = true;
            for (_, info) in self.sockets.iter() {
                let _ = self.poll.deregister(&info.sock);
            }
        }
    }

    fn accept_one(&mut self, mut msg: Conn<net::TcpStream>) {
        if self.backpressure {
            while !self.workers.is_empty() {
                match self.workers[self.next].send(msg) {
                    Ok(_) => (),
                    Err(err) => {
                        let _ = self.srv.unbounded_send(ServerCommand::WorkerDied(
                            self.workers[self.next].idx,
                        ));
                        msg = err.into_inner();
                        self.workers.swap_remove(self.next);
                        if self.workers.is_empty() {
                            error!("No workers");
                            return;
                        } else if self.workers.len() <= self.next {
                            self.next = 0;
                        }
                        continue;
                    }
                }
                self.next = (self.next + 1) % self.workers.len();
                break;
            }
        } else {
            let mut idx = 0;
            while idx < self.workers.len() {
                idx += 1;
                if self.workers[self.next].available() {
                    match self.workers[self.next].send(msg) {
                        Ok(_) => {
                            self.next = (self.next + 1) % self.workers.len();
                            return;
                        }
                        Err(err) => {
                            let _ = self.srv.unbounded_send(ServerCommand::WorkerDied(
                                self.workers[self.next].idx,
                            ));
                            msg = err.into_inner();
                            self.workers.swap_remove(self.next);
                            if self.workers.is_empty() {
                                error!("No workers");
                                self.backpressure(true);
                                return;
                            } else if self.workers.len() <= self.next {
                                self.next = 0;
                            }
                            continue;
                        }
                    }
                }
                self.next = (self.next + 1) % self.workers.len();
            }
            // enable backpressure
            self.backpressure(true);
            self.accept_one(msg);
        }
    }

    fn accept(&mut self, token: usize) {
        loop {
            let msg = if let Some(info) = self.sockets.get_mut(token) {
                match info.sock.accept_std() {
                    Ok((io, addr)) => Conn {
                        io,
                        token: info.token,
                        handler: info.handler,
                        peer: Some(addr),
                    },
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return,
                    Err(ref e) if connection_error(e) => continue,
                    Err(e) => {
                        error!("Error accepting connection: {}", e);
                        if let Err(err) = self.poll.deregister(&info.sock) {
                            error!("Can not deregister server socket {}", err);
                        }

                        // sleep after error
                        info.timeout = Some(Instant::now() + Duration::from_millis(500));

                        let r = self.timer.1.clone();
                        System::current().arbiter().do_send(Execute::new(
                            move || -> Result<(), ()> {
                                Arbiter::spawn(
                                    Delay::new(
                                        Instant::now() + Duration::from_millis(510),
                                    ).map_err(|_| ())
                                    .and_then(move |_| {
                                        let _ = r.set_readiness(mio::Ready::readable());
                                        Ok(())
                                    }),
                                );
                                Ok(())
                            },
                        ));
                        return;
                    }
                }
            } else {
                return;
            };

            self.accept_one(msg);
        }
    }
}

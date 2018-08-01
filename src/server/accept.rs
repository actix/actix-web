use std::sync::mpsc as sync_mpsc;
use std::time::{Duration, Instant};
use std::{io, net, thread};

use futures::{sync::mpsc, Future};
use mio;
use slab::Slab;
use tokio_timer::Delay;

use actix::{msgs::Execute, Arbiter, System};

use super::srv::{ServerCommand, Socket};
use super::worker::Conn;

pub(crate) enum Command {
    Pause,
    Resume,
    Stop,
    Worker(usize, mpsc::UnboundedSender<Conn<net::TcpStream>>),
}

struct ServerSocketInfo {
    addr: net::SocketAddr,
    token: usize,
    sock: mio::net::TcpListener,
    timeout: Option<Instant>,
}

struct Accept {
    poll: mio::Poll,
    rx: sync_mpsc::Receiver<Command>,
    sockets: Slab<ServerSocketInfo>,
    workers: Vec<(usize, mpsc::UnboundedSender<Conn<net::TcpStream>>)>,
    _reg: mio::Registration,
    next: usize,
    srv: mpsc::UnboundedSender<ServerCommand>,
    timer: (mio::Registration, mio::SetReadiness),
}

const CMD: mio::Token = mio::Token(0);
const TIMER: mio::Token = mio::Token(1);

pub(crate) fn start_accept_thread(
    socks: Vec<(usize, Socket)>, srv: mpsc::UnboundedSender<ServerCommand>,
    workers: Vec<(usize, mpsc::UnboundedSender<Conn<net::TcpStream>>)>,
) -> (mio::SetReadiness, sync_mpsc::Sender<Command>) {
    let (tx, rx) = sync_mpsc::channel();
    let (reg, readiness) = mio::Registration::new2();

    let sys = System::current();

    // start accept thread
    #[cfg_attr(feature = "cargo-clippy", allow(cyclomatic_complexity))]
    let _ = thread::Builder::new()
        .name("actix-web accept loop".to_owned())
        .spawn(move || {
            System::set_current(sys);
            Accept::new(reg, rx, socks, workers, srv).poll();
        });

    (readiness, tx)
}

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
    fn new(
        _reg: mio::Registration, rx: sync_mpsc::Receiver<Command>,
        socks: Vec<(usize, Socket)>,
        workers: Vec<(usize, mpsc::UnboundedSender<Conn<net::TcpStream>>)>,
        srv: mpsc::UnboundedSender<ServerCommand>,
    ) -> Accept {
        // Create a poll instance
        let poll = match mio::Poll::new() {
            Ok(poll) => poll,
            Err(err) => panic!("Can not create mio::Poll: {}", err),
        };

        // Start listening for incoming commands
        if let Err(err) =
            poll.register(&_reg, CMD, mio::Ready::readable(), mio::PollOpt::edge())
        {
            panic!("Can not register Registration: {}", err);
        }

        // Start accept
        let mut sockets = Slab::new();
        for (stoken, sock) in socks {
            let server = mio::net::TcpListener::from_std(sock.lst)
                .expect("Can not create mio::net::TcpListener");

            let entry = sockets.vacant_entry();
            let token = entry.key();

            // Start listening for incoming connections
            if let Err(err) = poll.register(
                &server,
                mio::Token(token + 1000),
                mio::Ready::readable(),
                mio::PollOpt::edge(),
            ) {
                panic!("Can not register io: {}", err);
            }

            entry.insert(ServerSocketInfo {
                token: stoken,
                addr: sock.addr,
                sock: server,
                timeout: None,
            });
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
            _reg,
            sockets,
            workers,
            srv,
            next: 0,
            timer: (tm, tmr),
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
                    _ => self.accept(token),
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
                        mio::Token(token + 1000),
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
                                mio::Token(token + 1000),
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
                    Command::Worker(idx, addr) => {
                        self.workers.push((idx, addr));
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

    fn accept(&mut self, token: mio::Token) {
        let token = usize::from(token);
        if token < 1000 {
            return;
        }

        if let Some(info) = self.sockets.get_mut(token - 1000) {
            loop {
                match info.sock.accept_std() {
                    Ok((io, addr)) => {
                        let mut msg = Conn {
                            io,
                            token: info.token,
                            peer: Some(addr),
                            http2: false,
                        };
                        while !self.workers.is_empty() {
                            match self.workers[self.next].1.unbounded_send(msg) {
                                Ok(_) => (),
                                Err(err) => {
                                    let _ = self.srv.unbounded_send(
                                        ServerCommand::WorkerDied(
                                            self.workers[self.next].0,
                                        ),
                                    );
                                    msg = err.into_inner();
                                    self.workers.swap_remove(self.next);
                                    if self.workers.is_empty() {
                                        error!("No workers");
                                        thread::sleep(Duration::from_millis(100));
                                        break;
                                    } else if self.workers.len() <= self.next {
                                        self.next = 0;
                                    }
                                    continue;
                                }
                            }
                            self.next = (self.next + 1) % self.workers.len();
                            break;
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
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
                                            let _ =
                                                r.set_readiness(mio::Ready::readable());
                                            Ok(())
                                        }),
                                );
                                Ok(())
                            },
                        ));
                        break;
                    }
                }
            }
        }
    }
}

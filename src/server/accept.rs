use std::sync::mpsc as sync_mpsc;
use std::time::Duration;
use std::{io, net, thread};

use futures::sync::mpsc;
use mio;
use slab::Slab;

#[cfg(feature = "tls")]
use native_tls::TlsAcceptor;

#[cfg(feature = "alpn")]
use openssl::ssl::{AlpnError, SslAcceptorBuilder};

#[cfg(feature = "rust-tls")]
use rustls::ServerConfig;

use super::srv::{ServerCommand, Socket};
use super::worker::{Conn, SocketInfo};

pub(crate) enum Command {
    Pause,
    Resume,
    Stop,
    Worker(usize, mpsc::UnboundedSender<Conn<net::TcpStream>>),
}

pub(crate) fn start_accept_thread(
    token: usize, sock: Socket, srv: mpsc::UnboundedSender<ServerCommand>,
    socks: Slab<SocketInfo>,
    mut workers: Vec<(usize, mpsc::UnboundedSender<Conn<net::TcpStream>>)>,
) -> (mio::SetReadiness, sync_mpsc::Sender<Command>) {
    let (tx, rx) = sync_mpsc::channel();
    let (reg, readiness) = mio::Registration::new2();

    // start accept thread
    #[cfg_attr(feature = "cargo-clippy", allow(cyclomatic_complexity))]
    let _ = thread::Builder::new()
        .name(format!("Accept on {}", sock.addr))
        .spawn(move || {
            const SRV: mio::Token = mio::Token(0);
            const CMD: mio::Token = mio::Token(1);

            let addr = sock.addr;
            let mut server = Some(
                mio::net::TcpListener::from_std(sock.lst)
                    .expect("Can not create mio::net::TcpListener"),
            );

            // Create a poll instance
            let poll = match mio::Poll::new() {
                Ok(poll) => poll,
                Err(err) => panic!("Can not create mio::Poll: {}", err),
            };

            // Start listening for incoming connections
            if let Some(ref srv) = server {
                if let Err(err) =
                    poll.register(srv, SRV, mio::Ready::readable(), mio::PollOpt::edge())
                {
                    panic!("Can not register io: {}", err);
                }
            }

            // Start listening for incoming commands
            if let Err(err) =
                poll.register(&reg, CMD, mio::Ready::readable(), mio::PollOpt::edge())
            {
                panic!("Can not register Registration: {}", err);
            }

            // Create storage for events
            let mut events = mio::Events::with_capacity(128);

            // Sleep on error
            let sleep = Duration::from_millis(100);

            let mut next = 0;
            loop {
                if let Err(err) = poll.poll(&mut events, None) {
                    panic!("Poll error: {}", err);
                }

                for event in events.iter() {
                    match event.token() {
                        SRV => if let Some(ref server) = server {
                            loop {
                                match server.accept_std() {
                                    Ok((io, addr)) => {
                                        let mut msg = Conn {
                                            io,
                                            token,
                                            peer: Some(addr),
                                            http2: false,
                                        };
                                        while !workers.is_empty() {
                                            match workers[next].1.unbounded_send(msg) {
                                                Ok(_) => (),
                                                Err(err) => {
                                                    let _ = srv.unbounded_send(
                                                        ServerCommand::WorkerDied(
                                                            workers[next].0,
                                                            socks.clone(),
                                                        ),
                                                    );
                                                    msg = err.into_inner();
                                                    workers.swap_remove(next);
                                                    if workers.is_empty() {
                                                        error!("No workers");
                                                        thread::sleep(sleep);
                                                        break;
                                                    } else if workers.len() <= next {
                                                        next = 0;
                                                    }
                                                    continue;
                                                }
                                            }
                                            next = (next + 1) % workers.len();
                                            break;
                                        }
                                    }
                                    Err(ref e)
                                        if e.kind() == io::ErrorKind::WouldBlock =>
                                    {
                                        break
                                    }
                                    Err(ref e) if connection_error(e) => continue,
                                    Err(e) => {
                                        error!("Error accepting connection: {}", e);
                                        // sleep after error
                                        thread::sleep(sleep);
                                        break;
                                    }
                                }
                            }
                        },
                        CMD => match rx.try_recv() {
                            Ok(cmd) => match cmd {
                                Command::Pause => if let Some(ref server) = server {
                                    if let Err(err) = poll.deregister(server) {
                                        error!(
                                            "Can not deregister server socket {}",
                                            err
                                        );
                                    } else {
                                        info!(
                                            "Paused accepting connections on {}",
                                            addr
                                        );
                                    }
                                },
                                Command::Resume => {
                                    if let Some(ref server) = server {
                                        if let Err(err) = poll.register(
                                            server,
                                            SRV,
                                            mio::Ready::readable(),
                                            mio::PollOpt::edge(),
                                        ) {
                                            error!("Can not resume socket accept process: {}", err);
                                        } else {
                                            info!("Accepting connections on {} has been resumed",
                                                  addr);
                                        }
                                    }
                                }
                                Command::Stop => {
                                    if let Some(server) = server.take() {
                                        let _ = poll.deregister(&server);
                                    }
                                    return;
                                }
                                Command::Worker(idx, addr) => {
                                    workers.push((idx, addr));
                                }
                            },
                            Err(err) => match err {
                                sync_mpsc::TryRecvError::Empty => (),
                                sync_mpsc::TryRecvError::Disconnected => {
                                    if let Some(server) = server.take() {
                                        let _ = poll.deregister(&server);
                                    }
                                    return;
                                }
                            },
                        },
                        _ => unreachable!(),
                    }
                }
            }
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

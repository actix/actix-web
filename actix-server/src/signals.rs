use std::io;

use actix_rt::spawn;
use futures::stream::futures_unordered;
use futures::{Async, Future, Poll, Stream};

use crate::server::Server;

/// Different types of process signals
#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Signal {
    /// SIGHUP
    Hup,
    /// SIGINT
    Int,
    /// SIGTERM
    Term,
    /// SIGQUIT
    Quit,
}

pub(crate) struct Signals {
    srv: Server,
    #[cfg(not(unix))]
    stream: SigStream,
    #[cfg(unix)]
    streams: Vec<SigStream>,
}

type SigStream = Box<Stream<Item = Signal, Error = io::Error>>;

impl Signals {
    pub(crate) fn start(srv: Server) {
        let fut = {
            #[cfg(not(unix))]
            {
                tokio_signal::ctrl_c().and_then(move |stream| Signals {
                    srv,
                    stream: Box::new(stream.map(|_| Signal::Int)),
                })
            }

            #[cfg(unix)]
            {
                use tokio_signal::unix;

                let mut sigs: Vec<Box<Future<Item = SigStream, Error = io::Error>>> =
                    Vec::new();
                sigs.push(Box::new(
                    tokio_signal::unix::Signal::new(tokio_signal::unix::SIGINT).map(|stream| {
                        let s: SigStream = Box::new(stream.map(|_| Signal::Int));
                        s
                    }),
                ));
                sigs.push(Box::new(
                    tokio_signal::unix::Signal::new(tokio_signal::unix::SIGHUP).map(
                        |stream: unix::Signal| {
                            let s: SigStream = Box::new(stream.map(|_| Signal::Hup));
                            s
                        },
                    ),
                ));
                sigs.push(Box::new(
                    tokio_signal::unix::Signal::new(tokio_signal::unix::SIGTERM).map(
                        |stream| {
                            let s: SigStream = Box::new(stream.map(|_| Signal::Term));
                            s
                        },
                    ),
                ));
                sigs.push(Box::new(
                    tokio_signal::unix::Signal::new(tokio_signal::unix::SIGQUIT).map(
                        |stream| {
                            let s: SigStream = Box::new(stream.map(|_| Signal::Quit));
                            s
                        },
                    ),
                ));
                futures_unordered(sigs)
                    .collect()
                    .map_err(|_| ())
                    .and_then(move |streams| Signals { srv, streams })
            }
        };
        spawn(fut);
    }
}

impl Future for Signals {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        #[cfg(not(unix))]
        loop {
            match self.stream.poll() {
                Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                Ok(Async::Ready(Some(sig))) => self.srv.signal(sig),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
            }
        }
        #[cfg(unix)]
        {
            for s in &mut self.streams {
                loop {
                    match s.poll() {
                        Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                        Ok(Async::NotReady) => break,
                        Ok(Async::Ready(Some(sig))) => self.srv.signal(sig),
                    }
                }
            }
            Ok(Async::NotReady)
        }
    }
}

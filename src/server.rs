use std::{io, mem, net};
use std::rc::Rc;
use std::time::Duration;
use std::collections::VecDeque;

use actix::dev::*;
use futures::{Future, Poll, Async};
use tokio_core::reactor::Timeout;
use tokio_core::net::{TcpListener, TcpStream};

use task::{Task, RequestInfo};
use router::Router;
use reader::{Reader, ReaderError};

/// An HTTP Server
pub struct HttpServer {
    router: Rc<Router>,
}

impl Actor for HttpServer {
    type Context = Context<Self>;
}

impl HttpServer {
    /// Create new http server with specified `RoutingMap`
    pub fn new(router: Router) -> Self {
        HttpServer {router: Rc::new(router)}
    }

    /// Start listening for incomming connections.
    pub fn serve<S, Addr>(self, addr: S) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let mut err = None;
        let mut addrs = Vec::new();
        if let Ok(iter) = addr.to_socket_addrs() {
            for addr in iter {
                match TcpListener::bind(&addr, Arbiter::handle()) {
                    Ok(tcp) => addrs.push(tcp),
                    Err(e) => err = Some(e),
                }
            }
        }
        if addrs.is_empty() {
            if let Some(e) = err.take() {
                Err(e)
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "Can not bind to address."))
            }
        } else {
            Ok(HttpServer::create(move |ctx| {
                for tcp in addrs {
                    ctx.add_stream(tcp.incoming());
                }
                self
            }))
        }
    }
}

impl ResponseType<(TcpStream, net::SocketAddr)> for HttpServer {
    type Item = ();
    type Error = ();
}

impl StreamHandler<(TcpStream, net::SocketAddr), io::Error> for HttpServer {}

impl Handler<(TcpStream, net::SocketAddr), io::Error> for HttpServer {

    fn handle(&mut self, msg: (TcpStream, net::SocketAddr), _: &mut Context<Self>)
              -> Response<Self, (TcpStream, net::SocketAddr)>
    {
        Arbiter::handle().spawn(
            HttpChannel{router: Rc::clone(&self.router),
                        addr: msg.1,
                        stream: msg.0,
                        reader: Reader::new(),
                        error: false,
                        items: VecDeque::new(),
                        inactive: Vec::new(),
                        keepalive: true,
                        keepalive_timer: None,
            });
        Self::empty()
    }
}


struct Entry {
    task: Task,
    req: RequestInfo,
    eof: bool,
    error: bool,
    finished: bool,
}

const KEEPALIVE_PERIOD: u64 = 15; // seconds
const MAX_PIPELINED_MESSAGES: usize = 16;

pub struct HttpChannel {
    router: Rc<Router>,
    #[allow(dead_code)]
    addr: net::SocketAddr,
    stream: TcpStream,
    reader: Reader,
    error: bool,
    items: VecDeque<Entry>,
    inactive: Vec<Entry>,
    keepalive: bool,
    keepalive_timer: Option<Timeout>,
}

impl Drop for HttpChannel {
    fn drop(&mut self) {
        println!("Drop http channel");
    }
}

impl Actor for HttpChannel {
    type Context = Context<Self>;
}

impl Future for HttpChannel {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // keep-alive timer
        if let Some(ref mut timeout) = self.keepalive_timer {
            match timeout.poll() {
                Ok(Async::Ready(_)) =>
                    return Ok(Async::Ready(())),
                Ok(Async::NotReady) => (),
                Err(_) => unreachable!(),
            }
        }

        loop {
            // check in-flight messages
            let mut idx = 0;
            while idx < self.items.len() {
                if idx == 0 {
                    if self.items[idx].error {
                        return Err(())
                    }

                    // this is anoying
                    let req: &RequestInfo = unsafe {
                        mem::transmute(&self.items[idx].req)
                    };
                    match self.items[idx].task.poll_io(&mut self.stream, req)
                    {
                        Ok(Async::Ready(val)) => {
                            let mut item = self.items.pop_front().unwrap();

                            // overide keep-alive state
                            if self.keepalive {
                                self.keepalive = item.task.keepalive();
                            }
                            if !val {
                                item.eof = true;
                                self.inactive.push(item);
                            }

                            // no keep-alive
                            if !self.keepalive && self.items.is_empty() {
                                return Ok(Async::Ready(()))
                            }
                            continue
                        },
                        Ok(Async::NotReady) => (),
                        Err(_) => {
                            // it is not possible to recover from error
                            // during task handling, so just drop connection
                            return Err(())
                        }
                    }
                } else if !self.items[idx].finished {
                    match self.items[idx].task.poll() {
                        Ok(Async::Ready(_)) =>
                            self.items[idx].finished = true,
                        Ok(Async::NotReady) => (),
                        Err(_) =>
                            self.items[idx].error = true,
                    }
                }
                idx += 1;
            }

            // read incoming data
            if !self.error && self.items.len() < MAX_PIPELINED_MESSAGES {
                match self.reader.parse(&mut self.stream) {
                    Ok(Async::Ready((req, payload))) => {
                        // stop keepalive timer
                        self.keepalive_timer.take();

                        // start request processing
                        let info = RequestInfo::new(&req);
                        self.items.push_back(
                            Entry {task: self.router.call(req, payload),
                                   req: info,
                                   eof: false,
                                   error: false,
                                   finished: false});
                    }
                    Err(err) => {
                        // kill keepalive
                        self.keepalive = false;
                        self.keepalive_timer.take();

                        // on parse error, stop reading stream but
                        // complete tasks
                        self.error = true;

                        if let ReaderError::Error(err) = err {
                            self.items.push_back(
                                Entry {task: Task::reply(err),
                                       req: RequestInfo::for_error(),
                                       eof: false,
                                       error: false,
                                       finished: false});
                        }
                    }
                    Ok(Async::NotReady) => {
                        // start keep-alive timer, this is also slow request timeout
                        if self.items.is_empty() {
                            if self.keepalive {
                                if self.keepalive_timer.is_none() {
                                    trace!("Start keep-alive timer");
                                    let mut timeout = Timeout::new(
                                        Duration::new(KEEPALIVE_PERIOD, 0),
                                        Arbiter::handle()).unwrap();
                                    // register timeout
                                    let _ = timeout.poll();
                                    self.keepalive_timer = Some(timeout);
                                }
                            } else {
                                // keep-alive disable, drop connection
                                return Ok(Async::Ready(()))
                            }
                        }
                        return Ok(Async::NotReady)
                    }
                }
            }

            // check for parse error
            if self.items.is_empty() && self.error {
                return Ok(Async::Ready(()))
            }
        }
    }
}

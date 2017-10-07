use std::{io, net};
use std::rc::Rc;
use std::collections::VecDeque;

use actix::dev::*;
use futures::{Future, Poll, Async};
use tokio_core::net::{TcpListener, TcpStream};

use task::Task;
use reader::Reader;
use router::{Router, RoutingMap};

/// An HTTP Server.
pub struct HttpServer {
    router: Rc<Router>,
}

impl Actor for HttpServer {
    type Context = Context<Self>;
}

impl HttpServer {
    /// Create new http server with specified `RoutingMap`
    pub fn new(routes: RoutingMap) -> Self {
        HttpServer {router: Rc::new(routes.into_router())}
    }

    /// Start listening for incomming connections.
    pub fn serve<Addr>(self, addr: &net::SocketAddr) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>
    {
        let tcp = TcpListener::bind(addr, Arbiter::handle())?;

        Ok(HttpServer::create(move |ctx| {
            ctx.add_stream(tcp.incoming());
            self
        }))
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
                        items: VecDeque::new(),
                        inactive: Vec::new(),
            });
        Response::Empty()
    }
}


struct Entry {
    task: Task,
    eof: bool,
    error: bool,
    finished: bool,
}

pub struct HttpChannel {
    router: Rc<Router>,
    #[allow(dead_code)]
    addr: net::SocketAddr,
    stream: TcpStream,
    reader: Reader,
    items: VecDeque<Entry>,
    inactive: Vec<Entry>,
}

impl Actor for HttpChannel {
    type Context = Context<Self>;
}

impl Future for HttpChannel {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            // check in-flight messages
            let mut idx = 0;
            while idx < self.items.len() {
                if idx == 0 {
                    if self.items[idx].error {
                        return Err(())
                    }
                    match self.items[idx].task.poll_io(&mut self.stream) {
                        Ok(Async::Ready(val)) => {
                            let mut item = self.items.pop_front().unwrap();
                            if !val {
                                item.eof = true;
                                self.inactive.push(item);
                            }
                            continue
                        },
                        Ok(Async::NotReady) => (),
                        Err(_) => return Err(()),
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
            match self.reader.parse(&mut self.stream) {
                Ok(Async::Ready((req, payload))) => {
                    self.items.push_back(
                        Entry {task: self.router.call(req, payload),
                               eof: false,
                               error: false,
                               finished: false});
                },
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Err(_) =>
                    return Err(()),
            }
        }
    }
}

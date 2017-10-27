use std::{io, net};
use std::rc::Rc;
use std::cell::UnsafeCell;
use std::time::Duration;
use std::marker::PhantomData;
use std::collections::VecDeque;

use actix::dev::*;
use futures::{Future, Poll, Async, Stream};
use tokio_core::reactor::Timeout;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_io::{AsyncRead, AsyncWrite};

use task::Task;
use reader::{Reader, ReaderError};
use payload::Payload;
use httpcodes::HTTPNotFound;
use httprequest::HttpRequest;

/// Low level http request handler
pub trait HttpHandler: 'static {
    /// Http handler prefix
    fn prefix(&self) -> &str;
    /// Handle request
    fn handle(&self, req: &mut HttpRequest, payload: Payload) -> Task;
}

/// An HTTP Server
///
/// `T` - async stream,  anything that implements `AsyncRead` + `AsyncWrite`.
///
/// `A` - peer address
///
/// `H` - request handler
pub struct HttpServer<T, A, H> {
    h: Rc<Vec<H>>,
    io: PhantomData<T>,
    addr: PhantomData<A>,
}

impl<T: 'static, A: 'static, H: 'static> Actor for HttpServer<T, A, H> {
    type Context = Context<Self>;
}

impl<T, A, H> HttpServer<T, A, H> where H: HttpHandler
{
    /// Create new http server with vec of http handlers
    pub fn new<U: IntoIterator<Item=H>>(handler: U) -> Self {
        let apps: Vec<_> = handler.into_iter().collect();

        HttpServer {h: Rc::new(apps),
                    io: PhantomData,
                    addr: PhantomData}
    }
}

impl<T, A, H> HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static,
          H: HttpHandler,
{
    /// Start listening for incomming connections from stream.
    pub fn serve_incoming<S, Addr>(self, stream: S) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: Stream<Item=(T, A), Error=io::Error> + 'static
    {
        Ok(HttpServer::create(move |ctx| {
            ctx.add_stream(stream.map(|(t, a)| IoStream(t, a)));
            self
        }))
    }
}

impl<H: HttpHandler> HttpServer<TcpStream, net::SocketAddr, H> {

    /// Start listening for incomming connections.
    ///
    /// This methods converts address to list of `SocketAddr`
    /// then binds to all available addresses.
    pub fn serve<S, Addr>(self, addr: S) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let mut err = None;
        let mut addrs = Vec::new();
        if let Ok(iter) = addr.to_socket_addrs() {
            for addr in iter {
                match TcpListener::bind(&addr, Arbiter::handle()) {
                    Ok(tcp) => addrs.push((addr, tcp)),
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
                for (addr, tcp) in addrs {
                    info!("Starting http server on {}", addr);
                    ctx.add_stream(tcp.incoming().map(|(t, a)| IoStream(t, a)));
                }
                self
            }))
        }
    }
}

struct IoStream<T, A>(T, A);

impl<T, A> ResponseType for IoStream<T, A>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static
{
    type Item = ();
    type Error = ();
}

impl<T, A, H> StreamHandler<IoStream<T, A>, io::Error> for HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static,
          H: HttpHandler + 'static {}

impl<T, A, H> Handler<IoStream<T, A>, io::Error> for HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static,
          H: HttpHandler + 'static,
{
    fn handle(&mut self, msg: IoStream<T, A>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T, A>>
    {
        Arbiter::handle().spawn(
            HttpChannel{router: Rc::clone(&self.h),
                        addr: msg.1,
                        stream: msg.0,
                        reader: Reader::new(),
                        error: false,
                        items: VecDeque::new(),
                        inactive: VecDeque::new(),
                        keepalive: true,
                        keepalive_timer: None,
            });
        Self::empty()
    }
}


struct Entry {
    task: Task,
    req: UnsafeCell<HttpRequest>,
    eof: bool,
    error: bool,
    finished: bool,
}

const KEEPALIVE_PERIOD: u64 = 15; // seconds
const MAX_PIPELINED_MESSAGES: usize = 16;

pub struct HttpChannel<T: 'static, A: 'static, H: 'static> {
    router: Rc<Vec<H>>,
    #[allow(dead_code)]
    addr: A,
    stream: T,
    reader: Reader,
    error: bool,
    items: VecDeque<Entry>,
    inactive: VecDeque<Entry>,
    keepalive: bool,
    keepalive_timer: Option<Timeout>,
}

impl<T: 'static, A: 'static, H: 'static> Drop for HttpChannel<T, A, H> {
    fn drop(&mut self) {
        println!("Drop http channel");
    }
}

impl<T, A, H> Actor for HttpChannel<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static,
          H: HttpHandler + 'static
{
    type Context = Context<Self>;
}

impl<T, A, H> Future for HttpChannel<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static,
          H: HttpHandler + 'static
{
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
            let mut not_ready = true;

            // check in-flight messages
            let mut idx = 0;
            while idx < self.items.len() {
                if idx == 0 {
                    if self.items[idx].error {
                        return Err(())
                    }

                    // this is anoying
                    let req = unsafe {self.items[idx].req.get().as_mut().unwrap()};
                    match self.items[idx].task.poll_io(&mut self.stream, req)
                    {
                        Ok(Async::Ready(ready)) => {
                            not_ready = false;
                            let mut item = self.items.pop_front().unwrap();

                            // overide keep-alive state
                            if self.keepalive {
                                self.keepalive = item.task.keepalive();
                            }
                            if !ready {
                                item.eof = true;
                                self.inactive.push_back(item);
                            }

                            // no keep-alive
                            if ready && !self.keepalive &&
                                self.items.is_empty() && self.inactive.is_empty()
                            {
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
                } else if !self.items[idx].finished && !self.items[idx].error {
                    match self.items[idx].task.poll() {
                        Ok(Async::NotReady) => (),
                        Ok(Async::Ready(_)) => {
                            not_ready = false;
                            self.items[idx].finished = true;
                        },
                        Err(_) =>
                            self.items[idx].error = true,
                    }
                }
                idx += 1;
            }

            // check inactive tasks
            let mut idx = 0;
            while idx < self.inactive.len() {
                if idx == 0 && self.inactive[idx].error && self.inactive[idx].finished {
                    let _ = self.inactive.pop_front();
                    continue
                }

                if !self.inactive[idx].finished && !self.inactive[idx].error {
                    match self.inactive[idx].task.poll() {
                        Ok(Async::NotReady) => (),
                        Ok(Async::Ready(_)) => {
                            not_ready = false;
                            self.inactive[idx].finished = true
                        }
                        Err(_) =>
                            self.inactive[idx].error = true,
                    }
                }
                idx += 1;
            }

            // read incoming data
            if !self.error && self.items.len() < MAX_PIPELINED_MESSAGES {
                match self.reader.parse(&mut self.stream) {
                    Ok(Async::Ready((mut req, payload))) => {
                        not_ready = false;

                        // stop keepalive timer
                        self.keepalive_timer.take();

                        // start request processing
                        let mut task = None;
                        for h in self.router.iter() {
                            if req.path().starts_with(h.prefix()) {
                                task = Some(h.handle(&mut req, payload));
                                break
                            }
                        }

                        self.items.push_back(
                            Entry {task: task.unwrap_or_else(|| Task::reply(HTTPNotFound)),
                                   req: UnsafeCell::new(req),
                                   eof: false,
                                   error: false,
                                   finished: false});
                    }
                    Err(err) => {
                        // notify all tasks
                        not_ready = false;
                        for entry in &mut self.items {
                            entry.task.disconnected()
                        }

                        // kill keepalive
                        self.keepalive = false;
                        self.keepalive_timer.take();

                        // on parse error, stop reading stream but
                        // tasks need to be completed
                        self.error = true;

                        if let ReaderError::Error(err) = err {
                            self.items.push_back(
                                Entry {task: Task::reply(err),
                                       req: UnsafeCell::new(HttpRequest::for_error()),
                                       eof: false,
                                       error: false,
                                       finished: false});
                        }
                    }
                    Ok(Async::NotReady) => {
                        // start keep-alive timer, this is also slow request timeout
                        if self.items.is_empty() && self.inactive.is_empty() {
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
            if self.items.is_empty() && self.inactive.is_empty() && self.error {
                return Ok(Async::Ready(()))
            }

            if not_ready {
                return Ok(Async::NotReady)
            }
        }
    }
}

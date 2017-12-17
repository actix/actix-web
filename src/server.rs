use std::{io, net, thread};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::marker::PhantomData;
use std::collections::HashMap;

use actix::dev::*;
use futures::Stream;
use futures::sync::mpsc;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::net::TcpStream;
use num_cpus;
use socket2::{Socket, Domain, Type};

#[cfg(feature="tls")]
use futures::{future, Future};
#[cfg(feature="tls")]
use native_tls::TlsAcceptor;
#[cfg(feature="tls")]
use tokio_tls::{TlsStream, TlsAcceptorExt};

#[cfg(feature="alpn")]
use futures::{future, Future};
#[cfg(feature="alpn")]
use openssl::ssl::{SslMethod, SslAcceptor, SslAcceptorBuilder};
#[cfg(feature="alpn")]
use openssl::pkcs12::ParsedPkcs12;
#[cfg(feature="alpn")]
use tokio_openssl::{SslStream, SslAcceptorExt};

use helpers;
use channel::{HttpChannel, HttpHandler, IntoHttpHandler};

/// Various server settings
#[derive(Debug, Clone)]
pub struct ServerSettings {
    addr: Option<net::SocketAddr>,
    secure: bool,
    host: String,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            addr: None,
            secure: false,
            host: "localhost:8080".to_owned(),
        }
    }
}

impl ServerSettings {
    /// Crate server settings instance
    fn new(addr: Option<net::SocketAddr>, secure: bool) -> Self {
        let host = if let Some(ref addr) = addr {
            format!("{}", addr)
        } else {
            "unknown".to_owned()
        };
        ServerSettings {
            addr: addr,
            secure: secure,
            host: host,
        }
    }

    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> Option<net::SocketAddr> {
        self.addr
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns host header value
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// An HTTP Server
///
/// `T` - async stream, anything that implements `AsyncRead` + `AsyncWrite`.
///
/// `A` - peer address
///
/// `H` - request handler
pub struct HttpServer<T, A, H, U>
    where H: 'static
{
    h: Option<Rc<WorkerSettings<H>>>,
    io: PhantomData<T>,
    addr: PhantomData<A>,
    threads: usize,
    backlog: i32,
    keep_alive: Option<u64>,
    factory: Arc<Fn() -> U + Send + Sync>,
    workers: Vec<SyncAddress<Worker<H>>>,
    sockets: HashMap<net::SocketAddr, Socket>,
}

impl<T: 'static, A: 'static, H, U: 'static> Actor for HttpServer<T, A, H, U> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<T: 'static, A: 'static, H, U: 'static>  HttpServer<T, A, H, U> {
    fn update_time(&self, ctx: &mut Context<Self>) {
        helpers::update_date();
        ctx.run_later(Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }
}

impl<T, A, H, U, V> HttpServer<T, A, H, U>
    where A: 'static,
          T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler,
          U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Create new http server with application factory
    pub fn new<F>(factory: F) -> Self
        where F: Sync + Send + 'static + Fn() -> U,
    {
        HttpServer{ h: None,
                    io: PhantomData,
                    addr: PhantomData,
                    threads: num_cpus::get(),
                    backlog: 2048,
                    keep_alive: None,
                    factory: Arc::new(factory),
                    workers: Vec::new(),
                    sockets: HashMap::new(),
        }
    }

    /// Set number of workers to start.
    ///
    /// By default http server uses number of available logical cpu as threads count.
    pub fn threads(mut self, num: usize) -> Self {
        self.threads = num;
        self
    }

    /// Set the maximum number of pending connections.
    ///
    /// This refers to the number of clients that can be waiting to be served.
    /// Exceeding this number results in the client getting an error when
    /// attempting to connect. It should only affect servers under significant load.
    ///
    /// Generally set in the 64-2048 range. Default value is 2048.
    pub fn backlog(mut self, num: i32) -> Self {
        self.backlog = num;
        self
    }

    /// Set server keep-alive setting.
    ///
    /// By default keep alive is enabled.
    ///
    ///  - `Some(75)` - enable
    ///
    ///  - `Some(0)` - disable
    ///
    ///  - `None` - use `SO_KEEPALIVE` socket option
    pub fn keep_alive(mut self, val: Option<u64>) -> Self {
        self.keep_alive = val;
        self
    }

    /// Start listening for incomming connections from a stream.
    ///
    /// This method uses only one thread for handling incoming connections.
    pub fn start_incoming<S, Addr>(mut self, stream: S, secure: bool) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: Stream<Item=(T, A), Error=io::Error> + 'static
    {
        if !self.sockets.is_empty() {
            let addrs: Vec<(net::SocketAddr, Socket)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), false);
            let workers = self.start_workers(&settings, &StreamHandlerType::Normal);

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting http server on {}", addr);
                start_accept_thread(sock, addr, workers.clone());
            }
        }

        // set server settings
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let settings = ServerSettings::new(Some(addr), secure);
        let mut apps: Vec<_> = (*self.factory)().into_iter().map(|h| h.into_handler()).collect();
        for app in &mut apps {
            app.server_settings(settings.clone());
        }
        self.h = Some(Rc::new(WorkerSettings::new(apps, self.keep_alive)));

        // start server
        Ok(HttpServer::create(move |ctx| {
            ctx.add_stream(stream.map(
                move |(t, _)| IoStream{io: t, peer: None, http2: false}));
            self
        }))
    }

    /// The socket address to bind
    ///
    /// To mind multiple addresses this method can be call multiple times.
    pub fn bind<S: net::ToSocketAddrs>(mut self, addr: S) -> io::Result<Self> {
        let mut err = None;
        let mut succ = false;
        if let Ok(iter) = addr.to_socket_addrs() {
            for addr in iter {
                let socket = match addr {
                    net::SocketAddr::V4(a) => {
                        let socket = Socket::new(Domain::ipv4(), Type::stream(), None)?;
                        match socket.bind(&a.into()) {
                            Ok(_) => socket,
                            Err(e) => {
                                err = Some(e);
                                continue;
                            }
                        }
                    }
                    net::SocketAddr::V6(a) => {
                        let socket = Socket::new(Domain::ipv6(), Type::stream(), None)?;
                        match socket.bind(&a.into()) {
                            Ok(_) => socket,
                            Err(e) => {
                                err = Some(e);
                                continue
                            }
                        }
                    }
                };
                succ = true;
                socket.listen(self.backlog)
                    .expect("failed to set socket backlog");
                socket.set_reuse_address(true)
                    .expect("failed to set socket reuse address");
                self.sockets.insert(addr, socket);
            }
        }

        if !succ {
            if let Some(e) = err.take() {
                Err(e)
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "Can not bind to address."))
            }
        } else {
            Ok(self)
        }
    }

    fn start_workers(&mut self, settings: &ServerSettings, handler: &StreamHandlerType)
                     -> Vec<mpsc::UnboundedSender<IoStream<Socket>>>
    {
        // start workers
        let mut workers = Vec::new();
        for _ in 0..self.threads {
            let s = settings.clone();
            let (tx, rx) = mpsc::unbounded::<IoStream<Socket>>();

            let h = handler.clone();
            let ka = self.keep_alive;
            let factory = Arc::clone(&self.factory);
            let addr = Arbiter::start(move |ctx: &mut Context<_>| {
                let mut apps: Vec<_> = (*factory)()
                    .into_iter().map(|h| h.into_handler()).collect();
                for app in &mut apps {
                    app.server_settings(s.clone());
                }
                ctx.add_stream(rx);
                Worker::new(apps, h, ka)
            });
            workers.push(tx);
            self.workers.push(addr);
        }
        info!("Starting {} http workers", self.threads);
        workers
    }
}

impl<H: HttpHandler, U, V> HttpServer<TcpStream, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming connections.
    ///
    /// This method starts number of http handler workers in seperate threads.
    /// For each address this method starts separate thread which does `accept()` in a loop.
    pub fn start(mut self) -> io::Result<SyncAddress<Self>>
    {
        if self.sockets.is_empty() {
            Err(io::Error::new(io::ErrorKind::Other, "No socket addresses are bound"))
        } else {
            let addrs: Vec<(net::SocketAddr, Socket)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), false);
            let workers = self.start_workers(&settings, &StreamHandlerType::Normal);

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting http server on {}", addr);
                start_accept_thread(sock, addr, workers.clone());
            }

            // start http server actor
            Ok(HttpServer::create(|_| {self}))
        }
    }
}

#[cfg(feature="tls")]
impl<H: HttpHandler, U, V> HttpServer<TlsStream<TcpStream>, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming tls connections.
    pub fn start_tls<Addr>(mut self, pkcs12: ::Pkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
    {
        if self.sockets.is_empty() {
            Err(io::Error::new(io::ErrorKind::Other, "No socket addresses are bound"))
        } else {
            let addrs: Vec<(net::SocketAddr, Socket)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), false);
            let acceptor = match TlsAcceptor::builder(pkcs12) {
                Ok(builder) => {
                    match builder.build() {
                        Ok(acceptor) => acceptor,
                        Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
                    }
                }
                Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
            };
            let workers = self.start_workers(&settings, &StreamHandlerType::Tls(acceptor));

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting tls http server on {}", addr);
                start_accept_thread(sock, addr, workers.clone());
            }

            // start http server actor
            Ok(HttpServer::create(|_| {self}))
        }
    }
}

#[cfg(feature="alpn")]
impl<H: HttpHandler, U, V> HttpServer<SslStream<TcpStream>, net::SocketAddr, H, U>
    where U: IntoIterator<Item=V> + 'static,
          V: IntoHttpHandler<Handler=H>,
{
    /// Start listening for incomming tls connections.
    ///
    /// This method sets alpn protocols to "h2" and "http/1.1"
    pub fn start_ssl<Addr>(mut self, identity: &ParsedPkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
    {
        if self.sockets.is_empty() {
            Err(io::Error::new(io::ErrorKind::Other, "No socket addresses are bound"))
        } else {
            let addrs: Vec<(net::SocketAddr, Socket)> = self.sockets.drain().collect();
            let settings = ServerSettings::new(Some(addrs[0].0), false);
            let acceptor = match SslAcceptorBuilder::mozilla_intermediate(
                SslMethod::tls(), &identity.pkey, &identity.cert, &identity.chain)
            {
                Ok(mut builder) => {
                    match builder.set_alpn_protocols(&[b"h2", b"http/1.1"]) {
                        Ok(_) => builder.build(),
                        Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err)),
                    }
                },
                Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
            };
            let workers = self.start_workers(&settings, &StreamHandlerType::Alpn(acceptor));

            // start acceptors threads
            for (addr, sock) in addrs {
                info!("Starting tls http server on {}", addr);
                start_accept_thread(sock, addr, workers.clone());
            }

            // start http server actor
            Ok(HttpServer::create(|_| {self}))
        }
    }
}

struct IoStream<T> {
    io: T,
    peer: Option<net::SocketAddr>,
    http2: bool,
}

impl<T> ResponseType for IoStream<T>
{
    type Item = ();
    type Error = ();
}

impl<T, A, H, U> StreamHandler<IoStream<T>, io::Error> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static {}

impl<T, A, H, U> Handler<IoStream<T>, io::Error> for HttpServer<T, A, H, U>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          U: 'static,
          A: 'static,
{
    fn error(&mut self, err: io::Error, _: &mut Context<Self>) {
        debug!("Error handling request: {}", err)
    }

    fn handle(&mut self, msg: IoStream<T>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T>>
    {
        Arbiter::handle().spawn(
            HttpChannel::new(Rc::clone(self.h.as_ref().unwrap()), msg.io, msg.peer, msg.http2));
        Self::empty()
    }
}

/// Http worker
///
/// Worker accepts Socket objects via unbounded channel and start requests processing.
struct Worker<H> {
    h: Rc<WorkerSettings<H>>,
    handler: StreamHandlerType,
}

pub(crate) struct WorkerSettings<H> {
    h: Vec<H>,
    enabled: bool,
    keep_alive: u64,
    bytes: Rc<helpers::SharedBytesPool>,
    messages: Rc<helpers::SharedMessagePool>,
}

impl<H> WorkerSettings<H> {
    pub(crate) fn new(h: Vec<H>, keep_alive: Option<u64>) -> WorkerSettings<H> {
        WorkerSettings {
            h: h,
            enabled: if let Some(ka) = keep_alive { ka > 0 } else { false },
            keep_alive: keep_alive.unwrap_or(0),
            bytes: Rc::new(helpers::SharedBytesPool::new()),
            messages: Rc::new(helpers::SharedMessagePool::new()),
        }
    }

    pub fn handlers(&self) -> &Vec<H> {
        &self.h
    }
    pub fn keep_alive(&self) -> u64 {
        self.keep_alive
    }
    pub fn keep_alive_enabled(&self) -> bool {
        self.enabled
    }
    pub fn get_shared_bytes(&self) -> helpers::SharedBytes {
        helpers::SharedBytes::new(self.bytes.get_bytes(), Rc::clone(&self.bytes))
    }
    pub fn get_http_message(&self) -> helpers::SharedHttpMessage {
        helpers::SharedHttpMessage::new(self.messages.get(), Rc::clone(&self.messages))
    }
}

impl<H: 'static> Worker<H> {

    fn new(h: Vec<H>, handler: StreamHandlerType, keep_alive: Option<u64>) -> Worker<H> {
        Worker {
            h: Rc::new(WorkerSettings::new(h, keep_alive)),
            handler: handler,
        }
    }
    
    fn update_time(&self, ctx: &mut Context<Self>) {
        helpers::update_date();
        ctx.run_later(Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }
}

impl<H: 'static> Actor for Worker<H> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<H> StreamHandler<IoStream<Socket>> for Worker<H>
    where H: HttpHandler + 'static {}

impl<H> Handler<IoStream<Socket>> for Worker<H>
    where H: HttpHandler + 'static,
{
    fn handle(&mut self, msg: IoStream<Socket>, _: &mut Context<Self>)
              -> Response<Self, IoStream<Socket>>
    {
        if !self.h.keep_alive_enabled() &&
            msg.io.set_keepalive(Some(Duration::new(75, 0))).is_err()
        {
            error!("Can not set socket keep-alive option");
        }
        self.handler.handle(Rc::clone(&self.h), msg);
        Self::empty()
    }
}

#[derive(Clone)]
enum StreamHandlerType {
    Normal,
    #[cfg(feature="tls")]
    Tls(TlsAcceptor),
    #[cfg(feature="alpn")]
    Alpn(SslAcceptor),
}

impl StreamHandlerType {

    fn handle<H: HttpHandler>(&mut self, h: Rc<WorkerSettings<H>>, msg: IoStream<Socket>) {
        match *self {
            StreamHandlerType::Normal => {
                let io = TcpStream::from_stream(msg.io.into_tcp_stream(), Arbiter::handle())
                    .expect("failed to associate TCP stream");

                Arbiter::handle().spawn(HttpChannel::new(h, io, msg.peer, msg.http2));
            }
            #[cfg(feature="tls")]
            StreamHandlerType::Tls(ref acceptor) => {
                let IoStream { io, peer, http2 } = msg;
                let io = TcpStream::from_stream(io.into_tcp_stream(), Arbiter::handle())
                    .expect("failed to associate TCP stream");

                Arbiter::handle().spawn(
                    TlsAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => Arbiter::handle().spawn(
                                HttpChannel::new(h, io, peer, http2)),
                            Err(err) =>
                                trace!("Error during handling tls connection: {}", err),
                        };
                        future::result(Ok(()))
                    })
                );
            }
            #[cfg(feature="alpn")]
            StreamHandlerType::Alpn(ref acceptor) => {
                let IoStream { io, peer, .. } = msg;
                let io = TcpStream::from_stream(io.into_tcp_stream(), Arbiter::handle())
                    .expect("failed to associate TCP stream");

                Arbiter::handle().spawn(
                    SslAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => {
                                let http2 = if let Some(p) = io.get_ref().ssl().selected_alpn_protocol()
                                {
                                    p.len() == 2 && &p == b"h2"
                                } else {
                                    false
                                };
                                Arbiter::handle().spawn(HttpChannel::new(h, io, peer, http2));
                            },
                            Err(err) =>
                                trace!("Error during handling tls connection: {}", err),
                        };
                        future::result(Ok(()))
                    })
                );
            }
        }
    }
}

fn start_accept_thread(sock: Socket, addr: net::SocketAddr,
                       workers: Vec<mpsc::UnboundedSender<IoStream<Socket>>>) {
    // start acceptors thread
    let _ = thread::Builder::new().name(format!("Accept on {}", addr)).spawn(move || {
        let mut next = 0;
        loop {
            match sock.accept() {
                Ok((socket, addr)) => {
                    let addr = if let Some(addr) = addr.as_inet() {
                        net::SocketAddr::V4(addr)
                    } else {
                        net::SocketAddr::V6(addr.as_inet6().unwrap())
                    };
                    let msg = IoStream{io: socket, peer: Some(addr), http2: false};
                    workers[next].unbounded_send(msg).expect("worker thread died");
                    next = (next + 1) % workers.len();
                }
                Err(err) => error!("Error accepting connection: {:?}", err),
            }
        }
    });
}

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{atomic::AtomicUsize, atomic::Ordering, Arc};
use std::{io, mem, net, time};

use futures::sync::mpsc::{unbounded, SendError, UnboundedSender};
use futures::sync::oneshot;
use futures::Future;
use net2::{TcpBuilder, TcpStreamExt};
use tokio::executor::current_thread;
use tokio_tcp::TcpStream;

use actix::msgs::StopArbiter;
use actix::{Actor, Addr, Arbiter, AsyncContext, Context, Handler, Message, Response};

use super::accept::AcceptNotify;
use super::channel::HttpChannel;
use super::settings::{ServerSettings, WorkerSettings};
use super::{
    AcceptorService, HttpHandler, IntoAsyncIo, IntoHttpHandler, IoStream, KeepAlive,
};

#[derive(Message)]
pub(crate) struct Conn<T> {
    pub io: T,
    pub token: Token,
    pub peer: Option<net::SocketAddr>,
    pub http2: bool,
}

#[derive(Clone, Copy)]
pub struct Token(usize);

impl Token {
    pub(crate) fn new(val: usize) -> Token {
        Token(val)
    }
}

pub(crate) struct Socket {
    pub lst: net::TcpListener,
    pub addr: net::SocketAddr,
    pub token: Token,
}

pub(crate) struct WorkerFactory<H: IntoHttpHandler + 'static> {
    pub factory: Arc<Fn() -> Vec<H> + Send + Sync>,
    pub host: Option<String>,
    pub keep_alive: KeepAlive,
    pub backlog: i32,
    sockets: Vec<Socket>,
    handlers: Vec<Box<IoStreamHandler<H::Handler, net::TcpStream>>>,
}

impl<H: IntoHttpHandler + 'static> WorkerFactory<H> {
    pub fn new<F>(factory: F) -> Self
    where
        F: Fn() -> Vec<H> + Send + Sync + 'static,
    {
        WorkerFactory {
            factory: Arc::new(factory),
            host: None,
            backlog: 2048,
            keep_alive: KeepAlive::Os,
            sockets: Vec::new(),
            handlers: Vec::new(),
        }
    }

    pub fn addrs(&self) -> Vec<net::SocketAddr> {
        self.sockets.iter().map(|s| s.addr).collect()
    }

    pub fn addrs_with_scheme(&self) -> Vec<(net::SocketAddr, &str)> {
        self.handlers
            .iter()
            .map(|s| (s.addr(), s.scheme()))
            .collect()
    }

    pub fn take_sockets(&mut self) -> Vec<Socket> {
        mem::replace(&mut self.sockets, Vec::new())
    }

    pub fn listen(&mut self, lst: net::TcpListener) {
        let token = Token(self.handlers.len());
        let addr = lst.local_addr().unwrap();
        self.handlers
            .push(Box::new(SimpleHandler::new(lst.local_addr().unwrap())));
        self.sockets.push(Socket { lst, addr, token })
    }

    pub fn listen_with<A>(&mut self, lst: net::TcpListener, acceptor: A)
    where
        A: AcceptorService<TcpStream> + Send + 'static,
    {
        let token = Token(self.handlers.len());
        let addr = lst.local_addr().unwrap();
        self.handlers.push(Box::new(StreamHandler::new(
            lst.local_addr().unwrap(),
            acceptor,
        )));
        self.sockets.push(Socket { lst, addr, token })
    }

    pub fn bind<S>(&mut self, addr: S) -> io::Result<()>
    where
        S: net::ToSocketAddrs,
    {
        let sockets = self.bind2(addr)?;

        for lst in sockets {
            let token = Token(self.handlers.len());
            let addr = lst.local_addr().unwrap();
            self.handlers
                .push(Box::new(SimpleHandler::new(lst.local_addr().unwrap())));
            self.sockets.push(Socket { lst, addr, token })
        }
        Ok(())
    }

    pub fn bind_with<S, A>(&mut self, addr: S, acceptor: &A) -> io::Result<()>
    where
        S: net::ToSocketAddrs,
        A: AcceptorService<TcpStream> + Send + 'static,
    {
        let sockets = self.bind2(addr)?;

        for lst in sockets {
            let token = Token(self.handlers.len());
            let addr = lst.local_addr().unwrap();
            self.handlers.push(Box::new(StreamHandler::new(
                lst.local_addr().unwrap(),
                acceptor.clone(),
            )));
            self.sockets.push(Socket { lst, addr, token })
        }
        Ok(())
    }

    fn bind2<S: net::ToSocketAddrs>(
        &self, addr: S,
    ) -> io::Result<Vec<net::TcpListener>> {
        let mut err = None;
        let mut succ = false;
        let mut sockets = Vec::new();
        for addr in addr.to_socket_addrs()? {
            match create_tcp_listener(addr, self.backlog) {
                Ok(lst) => {
                    succ = true;
                    sockets.push(lst);
                }
                Err(e) => err = Some(e),
            }
        }

        if !succ {
            if let Some(e) = err.take() {
                Err(e)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Can not bind to address.",
                ))
            }
        } else {
            Ok(sockets)
        }
    }

    pub fn start(
        &mut self, idx: usize, notify: AcceptNotify,
    ) -> (WorkerClient, Addr<Worker>) {
        let host = self.host.clone();
        let addr = self.handlers[0].addr();
        let factory = Arc::clone(&self.factory);
        let ka = self.keep_alive;
        let (tx, rx) = unbounded::<Conn<net::TcpStream>>();
        let client = WorkerClient::new(idx, tx);
        let conn = client.conn.clone();
        let sslrate = client.sslrate.clone();
        let handlers: Vec<_> = self.handlers.iter().map(|v| v.clone()).collect();

        let addr = Arbiter::start(move |ctx: &mut Context<_>| {
            let s = ServerSettings::new(Some(addr), &host, false);
            let apps: Vec<_> =
                (*factory)().into_iter().map(|h| h.into_handler()).collect();
            ctx.add_message_stream(rx);
            let inner = WorkerInner::new(apps, handlers, ka, s, conn, sslrate, notify);
            Worker {
                inner: Box::new(inner),
            }
        });

        (client, addr)
    }
}

#[derive(Clone)]
pub(crate) struct WorkerClient {
    pub idx: usize,
    tx: UnboundedSender<Conn<net::TcpStream>>,
    pub conn: Arc<AtomicUsize>,
    pub sslrate: Arc<AtomicUsize>,
}

impl WorkerClient {
    fn new(idx: usize, tx: UnboundedSender<Conn<net::TcpStream>>) -> Self {
        WorkerClient {
            idx,
            tx,
            conn: Arc::new(AtomicUsize::new(0)),
            sslrate: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn send(
        &self, msg: Conn<net::TcpStream>,
    ) -> Result<(), SendError<Conn<net::TcpStream>>> {
        self.tx.unbounded_send(msg)
    }

    pub fn available(&self, maxconn: usize, maxsslrate: usize) -> bool {
        if maxsslrate <= self.sslrate.load(Ordering::Relaxed) {
            false
        } else {
            maxconn > self.conn.load(Ordering::Relaxed)
        }
    }
}

/// Stop worker message. Returns `true` on successful shutdown
/// and `false` if some connections still alive.
pub(crate) struct StopWorker {
    pub graceful: Option<time::Duration>,
}

impl Message for StopWorker {
    type Result = Result<bool, ()>;
}

/// Http worker
///
/// Worker accepts Socket objects via unbounded channel and start requests
/// processing.
pub(crate) struct Worker {
    inner: Box<WorkerHandler>,
}

impl Actor for Worker {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_date(ctx);
    }
}

impl Worker {
    fn update_date(&self, ctx: &mut Context<Self>) {
        self.inner.update_date();
        ctx.run_later(time::Duration::new(1, 0), |slf, ctx| slf.update_date(ctx));
    }

    fn shutdown_timeout(
        &self, ctx: &mut Context<Worker>, tx: oneshot::Sender<bool>, dur: time::Duration,
    ) {
        // sleep for 1 second and then check again
        ctx.run_later(time::Duration::new(1, 0), move |slf, ctx| {
            let num = slf.inner.num_channels();
            if num == 0 {
                let _ = tx.send(true);
                Arbiter::current().do_send(StopArbiter(0));
            } else if let Some(d) = dur.checked_sub(time::Duration::new(1, 0)) {
                slf.shutdown_timeout(ctx, tx, d);
            } else {
                info!("Force shutdown http worker, {} connections", num);
                slf.inner.force_shutdown();
                let _ = tx.send(false);
                Arbiter::current().do_send(StopArbiter(0));
            }
        });
    }
}

impl Handler<Conn<net::TcpStream>> for Worker {
    type Result = ();

    fn handle(&mut self, msg: Conn<net::TcpStream>, _: &mut Context<Self>) {
        self.inner.handle_connect(msg)
    }
}

/// `StopWorker` message handler
impl Handler<StopWorker> for Worker {
    type Result = Response<bool, ()>;

    fn handle(&mut self, msg: StopWorker, ctx: &mut Context<Self>) -> Self::Result {
        let num = self.inner.num_channels();
        if num == 0 {
            info!("Shutting down http worker, 0 connections");
            Response::reply(Ok(true))
        } else if let Some(dur) = msg.graceful {
            info!("Graceful http worker shutdown, {} connections", num);
            let (tx, rx) = oneshot::channel();
            self.shutdown_timeout(ctx, tx, dur);
            Response::async(rx.map_err(|_| ()))
        } else {
            info!("Force shutdown http worker, {} connections", num);
            self.inner.force_shutdown();
            Response::reply(Ok(false))
        }
    }
}

trait WorkerHandler {
    fn update_date(&self);

    fn handle_connect(&mut self, Conn<net::TcpStream>);

    fn force_shutdown(&self);

    fn num_channels(&self) -> usize;
}

struct WorkerInner<H>
where
    H: HttpHandler + 'static,
{
    settings: Rc<WorkerSettings<H>>,
    socks: Vec<Box<IoStreamHandler<H, net::TcpStream>>>,
    tcp_ka: Option<time::Duration>,
}

impl<H: HttpHandler + 'static> WorkerInner<H> {
    pub(crate) fn new(
        h: Vec<H>, socks: Vec<Box<IoStreamHandler<H, net::TcpStream>>>,
        keep_alive: KeepAlive, settings: ServerSettings, conn: Arc<AtomicUsize>,
        sslrate: Arc<AtomicUsize>, notify: AcceptNotify,
    ) -> WorkerInner<H> {
        let tcp_ka = if let KeepAlive::Tcp(val) = keep_alive {
            Some(time::Duration::new(val as u64, 0))
        } else {
            None
        };

        WorkerInner {
            settings: Rc::new(WorkerSettings::new(
                h, keep_alive, settings, notify, conn, sslrate,
            )),
            socks,
            tcp_ka,
        }
    }
}

impl<H> WorkerHandler for WorkerInner<H>
where
    H: HttpHandler + 'static,
{
    fn update_date(&self) {
        self.settings.update_date();
    }

    fn handle_connect(&mut self, msg: Conn<net::TcpStream>) {
        if self.tcp_ka.is_some() && msg.io.set_keepalive(self.tcp_ka).is_err() {
            error!("Can not set socket keep-alive option");
        }
        self.socks[msg.token.0].handle(Rc::clone(&self.settings), msg.io, msg.peer);
    }

    fn num_channels(&self) -> usize {
        self.settings.num_channels()
    }

    fn force_shutdown(&self) {
        self.settings.head().traverse::<TcpStream, H>();
    }
}

struct SimpleHandler<Io> {
    addr: net::SocketAddr,
    io: PhantomData<Io>,
}

impl<Io: IntoAsyncIo> Clone for SimpleHandler<Io> {
    fn clone(&self) -> Self {
        SimpleHandler {
            addr: self.addr,
            io: PhantomData,
        }
    }
}

impl<Io: IntoAsyncIo> SimpleHandler<Io> {
    fn new(addr: net::SocketAddr) -> Self {
        SimpleHandler {
            addr,
            io: PhantomData,
        }
    }
}

impl<H, Io> IoStreamHandler<H, Io> for SimpleHandler<Io>
where
    H: HttpHandler,
    Io: IntoAsyncIo + Send + 'static,
    Io::Io: IoStream,
{
    fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    fn clone(&self) -> Box<IoStreamHandler<H, Io>> {
        Box::new(Clone::clone(self))
    }

    fn scheme(&self) -> &'static str {
        "http"
    }

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>) {
        let mut io = match io.into_async_io() {
            Ok(io) => io,
            Err(err) => {
                trace!("Failed to create async io: {}", err);
                return;
            }
        };
        let _ = io.set_nodelay(true);

        current_thread::spawn(HttpChannel::new(h, io, peer, false));
    }
}

struct StreamHandler<A, Io> {
    acceptor: A,
    addr: net::SocketAddr,
    io: PhantomData<Io>,
}

impl<Io: IntoAsyncIo, A: AcceptorService<Io::Io>> StreamHandler<A, Io> {
    fn new(addr: net::SocketAddr, acceptor: A) -> Self {
        StreamHandler {
            addr,
            acceptor,
            io: PhantomData,
        }
    }
}

impl<Io: IntoAsyncIo, A: AcceptorService<Io::Io>> Clone for StreamHandler<A, Io> {
    fn clone(&self) -> Self {
        StreamHandler {
            addr: self.addr,
            acceptor: self.acceptor.clone(),
            io: PhantomData,
        }
    }
}

impl<H, Io, A> IoStreamHandler<H, Io> for StreamHandler<A, Io>
where
    H: HttpHandler,
    Io: IntoAsyncIo + Send + 'static,
    Io::Io: IoStream,
    A: AcceptorService<Io::Io> + Send + 'static,
{
    fn addr(&self) -> net::SocketAddr {
        self.addr
    }

    fn clone(&self) -> Box<IoStreamHandler<H, Io>> {
        Box::new(Clone::clone(self))
    }

    fn scheme(&self) -> &'static str {
        self.acceptor.scheme()
    }

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>) {
        let mut io = match io.into_async_io() {
            Ok(io) => io,
            Err(err) => {
                trace!("Failed to create async io: {}", err);
                return;
            }
        };
        let _ = io.set_nodelay(true);

        h.conn_rate_add();
        current_thread::spawn(self.acceptor.accept(io).then(move |res| {
            h.conn_rate_del();
            match res {
                Ok(io) => current_thread::spawn(HttpChannel::new(h, io, peer, false)),
                Err(err) => trace!("Can not establish connection: {}", err),
            }
            Ok(())
        }))
    }
}

impl<H, Io: 'static> IoStreamHandler<H, Io> for Box<IoStreamHandler<H, Io>>
where
    H: HttpHandler,
    Io: IntoAsyncIo,
{
    fn addr(&self) -> net::SocketAddr {
        self.as_ref().addr()
    }

    fn clone(&self) -> Box<IoStreamHandler<H, Io>> {
        self.as_ref().clone()
    }

    fn scheme(&self) -> &'static str {
        self.as_ref().scheme()
    }

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>) {
        self.as_ref().handle(h, io, peer)
    }
}

pub(crate) trait IoStreamHandler<H, Io>: Send
where
    H: HttpHandler,
{
    fn clone(&self) -> Box<IoStreamHandler<H, Io>>;

    fn addr(&self) -> net::SocketAddr;

    fn scheme(&self) -> &'static str;

    fn handle(&self, h: Rc<WorkerSettings<H>>, io: Io, peer: Option<net::SocketAddr>);
}

fn create_tcp_listener(
    addr: net::SocketAddr, backlog: i32,
) -> io::Result<net::TcpListener> {
    let builder = match addr {
        net::SocketAddr::V4(_) => TcpBuilder::new_v4()?,
        net::SocketAddr::V6(_) => TcpBuilder::new_v6()?,
    };
    builder.reuse_address(true)?;
    builder.bind(addr)?;
    Ok(builder.listen(backlog)?)
}

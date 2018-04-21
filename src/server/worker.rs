use std::rc::Rc;
use std::{marker, net, time};

use futures::task::current as current_task;
use futures::unsync::oneshot;
use futures::{Async, Future, Poll};
use net2::TcpStreamExt;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;

#[cfg(any(feature = "tls", feature = "alpn"))]
use futures::future;

#[cfg(feature = "tls")]
use native_tls::TlsAcceptor;
#[cfg(feature = "tls")]
use tokio_tls::TlsAcceptorExt;

#[cfg(feature = "alpn")]
use openssl::ssl::SslAcceptor;
#[cfg(feature = "alpn")]
use tokio_openssl::SslAcceptorExt;

use actix::msgs::StopArbiter;
use actix::*;

//use server::channel::HttpChannel;
use server::h1;
use server::h1decoder::Message as H1Message;
use server::settings::WorkerSettings;
use server::{HttpHandler, KeepAlive};

use io::{IoChannel, IoCommand, TaskCommand};

#[derive(Message)]
pub(crate) struct Conn<T> {
    pub io: T,
    pub peer: Option<net::SocketAddr>,
    pub http2: bool,
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
pub(crate) struct Worker<H>
where
    H: HttpHandler + 'static,
{
    settings: Rc<WorkerSettings<H>>,
    hnd: Handle,
    handler: StreamHandlerType,
    tcp_ka: Option<time::Duration>,
    io: IoWriter,
}

impl<H: HttpHandler + 'static> Worker<H> {
    pub(crate) fn new(
        h: Vec<H>, handler: StreamHandlerType, keep_alive: KeepAlive,
    ) -> Worker<H> {
        let tcp_ka = if let KeepAlive::Tcp(val) = keep_alive {
            Some(time::Duration::new(val as u64, 0))
        } else {
            None
        };
        let io = IoWriter {
            inner: Rc::new(IoChannel::new()),
        };

        Worker {
            settings: Rc::new(WorkerSettings::new(h, keep_alive)),
            hnd: Arbiter::handle().clone(),
            io,
            handler,
            tcp_ka,
        }
    }

    fn update_time(&self, ctx: &mut Context<Self>) {
        self.settings.update_date();
        ctx.run_later(time::Duration::new(1, 0), |slf, ctx| {
            slf.update_time(ctx)
        });
    }

    fn shutdown_timeout(
        &self, ctx: &mut Context<Self>, tx: oneshot::Sender<bool>, dur: time::Duration,
    ) {
        // sleep for 1 second and then check again
        ctx.run_later(time::Duration::new(1, 0), move |slf, ctx| {
            let num = slf.settings.num_channels();
            if num == 0 {
                let _ = tx.send(true);
                Arbiter::arbiter().do_send(StopArbiter(0));
            } else if let Some(d) = dur.checked_sub(time::Duration::new(1, 0)) {
                slf.shutdown_timeout(ctx, tx, d);
            } else {
                info!("Force shutdown http worker, {} connections", num);
                slf.settings.head().traverse::<TcpStream, H>();
                let _ = tx.send(false);
                Arbiter::arbiter().do_send(StopArbiter(0));
            }
        });
    }
}

impl<H: 'static> Actor for Worker<H>
where
    H: HttpHandler + 'static,
{
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
        self.io.inner.set_notify(current_task());
        ctx.spawn(IoQueue(marker::PhantomData));
    }
}

impl<H> Handler<Conn<net::TcpStream>> for Worker<H>
where
    H: HttpHandler + 'static,
{
    type Result = ();

    fn handle(&mut self, msg: Conn<net::TcpStream>, _: &mut Context<Self>) {
        if self.tcp_ka.is_some() && msg.io.set_keepalive(self.tcp_ka).is_err() {
            error!("Can not set socket keep-alive option");
        }
        //println!("HANDLE: {:?}", msg.io);
        self.io.inner.add_source(msg.io, msg.peer, msg.http2);
        //self.handler
        //.handle(Rc::clone(&self.settings), &self.hnd, msg);
    }
}

/// `StopWorker` message handler
impl<H> Handler<StopWorker> for Worker<H>
where
    H: HttpHandler + 'static,
{
    type Result = Response<bool, ()>;

    fn handle(&mut self, msg: StopWorker, ctx: &mut Context<Self>) -> Self::Result {
        let num = self.settings.num_channels();
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
            self.settings.head().traverse::<TcpStream, H>();
            Response::reply(Ok(false))
        }
    }
}

#[derive(Clone)]
pub(crate) enum StreamHandlerType {
    Normal,
    #[cfg(feature = "tls")]
    Tls(TlsAcceptor),
    #[cfg(feature = "alpn")]
    Alpn(SslAcceptor),
}

/*
impl StreamHandlerType {
    fn handle<H: HttpHandler>(
        &mut self, h: Rc<WorkerSettings<H>>, hnd: &Handle, msg: Conn<net::TcpStream>,
    ) {
        match *self {
            StreamHandlerType::Normal => {
                let _ = msg.io.set_nodelay(true);
                let io = TcpStream::from_stream(msg.io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(HttpChannel::new(h, io, msg.peer, msg.http2));
            }
            #[cfg(feature = "tls")]
            StreamHandlerType::Tls(ref acceptor) => {
                let Conn { io, peer, http2 } = msg;
                let _ = io.set_nodelay(true);
                let io = TcpStream::from_stream(io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(
                    TlsAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => Arbiter::handle()
                                .spawn(HttpChannel::new(h, io, peer, http2)),
                            Err(err) => {
                                trace!("Error during handling tls connection: {}", err)
                            }
                        };
                        future::result(Ok(()))
                    }),
                );
            }
            #[cfg(feature = "alpn")]
            StreamHandlerType::Alpn(ref acceptor) => {
                let Conn { io, peer, .. } = msg;
                let _ = io.set_nodelay(true);
                let io = TcpStream::from_stream(io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(
                    SslAcceptorExt::accept_async(acceptor, io).then(move |res| {
                        match res {
                            Ok(io) => {
                                let http2 = if let Some(p) =
                                    io.get_ref().ssl().selected_alpn_protocol()
                                {
                                    p.len() == 2 && &p == b"h2"
                                } else {
                                    false
                                };
                                Arbiter::handle()
                                    .spawn(HttpChannel::new(h, io, peer, http2));
                            }
                            Err(err) => {
                                trace!("Error during handling tls connection: {}", err)
                            }
                        };
                        future::result(Ok(()))
                    }),
                );
            }
        }
    }
}*/

struct IoQueue<H>(marker::PhantomData<H>);

impl<H: HttpHandler + 'static> fut::ActorFuture for IoQueue<H> {
    type Item = ();
    type Error = ();
    type Actor = Worker<H>;

    fn poll(&mut self, act: &mut Worker<H>, _: &mut Context<Worker<H>>) -> Poll<(), ()> {
        act.io.inner.as_ref().start();

        loop {
            match act.io.inner.try_recv() {
                Ok(TaskCommand::Stream(stream)) => {
                    act.hnd.spawn(h1::Http1::new(
                        Rc::clone(&act.settings),
                        stream,
                        act.io.clone(),
                    ));
                }
                Err(_) => break,
            }
        }
        act.io.inner.as_ref().end();

        Ok(Async::NotReady)
    }
}

#[derive(Clone)]
pub(crate) struct IoWriter {
    pub inner: Rc<IoChannel>,
}

impl IoWriter {
    pub fn send(&self, msg: IoCommand) {
        self.inner.send(msg)
    }
}

use std::{net, time};
use std::rc::Rc;
use std::cell::{Cell, RefCell, RefMut};
use futures::Future;
use futures::unsync::oneshot;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use net2::TcpStreamExt;

#[cfg(feature="tls")]
use futures::future;
#[cfg(feature="tls")]
use native_tls::TlsAcceptor;
#[cfg(feature="tls")]
use tokio_tls::TlsAcceptorExt;

#[cfg(feature="alpn")]
use futures::future;
#[cfg(feature="alpn")]
use openssl::ssl::SslAcceptor;
#[cfg(feature="alpn")]
use tokio_openssl::SslAcceptorExt;

use actix::*;
use actix::msgs::StopArbiter;

use helpers;
use channel::{HttpChannel, HttpHandler};


#[derive(Message)]
pub(crate) struct Conn<T> {
    pub io: T,
    pub peer: Option<net::SocketAddr>,
    pub http2: bool,
}

/// Stop worker message. Returns `true` on successful shutdown
/// and `false` if some connections still alive.
#[derive(Message)]
#[rtype(bool)]
pub(crate) struct StopWorker {
    pub graceful: Option<time::Duration>,
}

pub(crate) struct WorkerSettings<H> {
    h: RefCell<Vec<H>>,
    enabled: bool,
    keep_alive: u64,
    bytes: Rc<helpers::SharedBytesPool>,
    messages: Rc<helpers::SharedMessagePool>,
    channels: Cell<usize>,
}

impl<H> WorkerSettings<H> {
    pub(crate) fn new(h: Vec<H>, keep_alive: Option<u64>) -> WorkerSettings<H> {
        WorkerSettings {
            h: RefCell::new(h),
            enabled: if let Some(ka) = keep_alive { ka > 0 } else { false },
            keep_alive: keep_alive.unwrap_or(0),
            bytes: Rc::new(helpers::SharedBytesPool::new()),
            messages: Rc::new(helpers::SharedMessagePool::new()),
            channels: Cell::new(0),
        }
    }

    pub fn handlers(&self) -> RefMut<Vec<H>> {
        self.h.borrow_mut()
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
    pub fn add_channel(&self) {
        self.channels.set(self.channels.get()+1);
    }
    pub fn remove_channel(&self) {
        let num = self.channels.get();
        if num > 0 {
            self.channels.set(num-1);
        } else {
            error!("Number of removed channels is bigger than added channel. Bug in actix-web");
        }
    }
}

/// Http worker
///
/// Worker accepts Socket objects via unbounded channel and start requests processing.
pub(crate) struct Worker<H> {
    h: Rc<WorkerSettings<H>>,
    hnd: Handle,
    handler: StreamHandlerType,
}

impl<H: 'static> Worker<H> {

    pub(crate) fn new(h: Vec<H>, handler: StreamHandlerType, keep_alive: Option<u64>)
                      -> Worker<H>
    {
        Worker {
            h: Rc::new(WorkerSettings::new(h, keep_alive)),
            hnd: Arbiter::handle().clone(),
            handler: handler,
        }
    }

    fn update_time(&self, ctx: &mut Context<Self>) {
        helpers::update_date();
        ctx.run_later(time::Duration::new(1, 0), |slf, ctx| slf.update_time(ctx));
    }

    fn shutdown_timeout(&self, ctx: &mut Context<Self>,
                        tx: oneshot::Sender<bool>, dur: time::Duration) {
        // sleep for 1 second and then check again
        ctx.run_later(time::Duration::new(1, 0), move |slf, ctx| {
            let num = slf.h.channels.get();
            if num == 0 {
                let _ = tx.send(true);
                Arbiter::arbiter().send(StopArbiter(0));
            } else if let Some(d) = dur.checked_sub(time::Duration::new(1, 0)) {
                slf.shutdown_timeout(ctx, tx, d);
            } else {
                info!("Force shutdown http worker, {} connections", num);
                let _ = tx.send(false);
                Arbiter::arbiter().send(StopArbiter(0));
            }
        });
    }
}

impl<H: 'static> Actor for Worker<H> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.update_time(ctx);
    }
}

impl<H> StreamHandler<Conn<net::TcpStream>> for Worker<H>
    where H: HttpHandler + 'static {}

impl<H> Handler<Conn<net::TcpStream>> for Worker<H>
    where H: HttpHandler + 'static,
{
    fn handle(&mut self, msg: Conn<net::TcpStream>, _: &mut Context<Self>)
              -> Response<Self, Conn<net::TcpStream>>
    {
        if !self.h.keep_alive_enabled() &&
            msg.io.set_keepalive(Some(time::Duration::new(75, 0))).is_err()
        {
            error!("Can not set socket keep-alive option");
        }
        self.handler.handle(Rc::clone(&self.h), &self.hnd, msg);
        Self::empty()
    }
}

/// `StopWorker` message handler
impl<H> Handler<StopWorker> for Worker<H>
    where H: HttpHandler + 'static,
{
    fn handle(&mut self, msg: StopWorker, ctx: &mut Context<Self>) -> Response<Self, StopWorker>
    {
        let num = self.h.channels.get();
        if num == 0 {
            info!("Shutting down http worker, 0 connections");
            Self::reply(true)
        } else if let Some(dur) = msg.graceful {
            info!("Graceful http worker shutdown, {} connections", num);
            let (tx, rx) = oneshot::channel();
            self.shutdown_timeout(ctx, tx, dur);
            Self::async_reply(rx.map_err(|_| ()).actfuture())
        } else {
            info!("Force shutdown http worker, {} connections", num);
            Self::reply(false)
        }
    }
}

#[derive(Clone)]
pub(crate) enum StreamHandlerType {
    Normal,
    #[cfg(feature="tls")]
    Tls(TlsAcceptor),
    #[cfg(feature="alpn")]
    Alpn(SslAcceptor),
}

impl StreamHandlerType {

    fn handle<H: HttpHandler>(&mut self,
                              h: Rc<WorkerSettings<H>>,
                              hnd: &Handle, msg: Conn<net::TcpStream>) {
        match *self {
            StreamHandlerType::Normal => {
                let io = TcpStream::from_stream(msg.io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(HttpChannel::new(h, io, msg.peer, msg.http2));
            }
            #[cfg(feature="tls")]
            StreamHandlerType::Tls(ref acceptor) => {
                let Conn { io, peer, http2 } = msg;
                let io = TcpStream::from_stream(io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(
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
                let Conn { io, peer, .. } = msg;
                let io = TcpStream::from_stream(io, hnd)
                    .expect("failed to associate TCP stream");

                hnd.spawn(
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

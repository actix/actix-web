use std::marker::PhantomData;
use std::net;
use std::time::Duration;

use actix_net::service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Poll};

use super::channel::HttpChannel;
use super::handler::{HttpHandler, IntoHttpHandler};
use super::settings::{ServerSettings, WorkerSettings};
use super::{IoStream, KeepAlive};

pub enum HttpServiceMessage<T> {
    /// New stream
    Connect(T),
    /// Gracefull shutdown
    Shutdown(Duration),
    /// Force shutdown
    ForceShutdown,
}

pub(crate) struct HttpService<F, H, Io>
where
    F: Fn() -> H,
    H: IntoHttpHandler,
    Io: IoStream,
{
    factory: F,
    addr: net::SocketAddr,
    host: Option<String>,
    keep_alive: KeepAlive,
    _t: PhantomData<Io>,
}

impl<F, H, Io> HttpService<F, H, Io>
where
    F: Fn() -> H,
    H: IntoHttpHandler,
    Io: IoStream,
{
    pub fn new(
        factory: F, addr: net::SocketAddr, host: Option<String>, keep_alive: KeepAlive,
    ) -> Self {
        HttpService {
            factory,
            addr,
            host,
            keep_alive,
            _t: PhantomData,
        }
    }
}

impl<F, H, Io> NewService for HttpService<F, H, Io>
where
    F: Fn() -> H,
    H: IntoHttpHandler,
    Io: IoStream,
{
    type Request = Io;
    type Response = ();
    type Error = ();
    type InitError = ();
    type Service = HttpServiceHandler<H::Handler, Io>;
    type Future = FutureResult<Self::Service, Self::Error>;

    fn new_service(&self) -> Self::Future {
        let s = ServerSettings::new(Some(self.addr), &self.host, false);
        let app = (self.factory)().into_handler();

        ok(HttpServiceHandler::new(app, self.keep_alive, s))
    }
}

pub(crate) struct HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    settings: WorkerSettings<H>,
    tcp_ka: Option<Duration>,
    _t: PhantomData<Io>,
}

impl<H, Io> HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    fn new(
        app: H, keep_alive: KeepAlive, settings: ServerSettings,
    ) -> HttpServiceHandler<H, Io> {
        let tcp_ka = if let KeepAlive::Tcp(val) = keep_alive {
            Some(Duration::new(val as u64, 0))
        } else {
            None
        };
        let settings = WorkerSettings::new(app, keep_alive, settings);

        HttpServiceHandler {
            tcp_ka,
            settings,
            _t: PhantomData,
        }
    }
}

impl<H, Io> Service for HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    type Request = Io;
    type Response = ();
    type Error = ();
    type Future = HttpChannel<Io, H>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }

    fn call(&mut self, mut req: Self::Request) -> Self::Future {
        let _ = req.set_nodelay(true);
        HttpChannel::new(self.settings.clone(), req, None)
    }

    // fn shutdown(&self, force: bool) {
    //     if force {
    //         self.settings.head().traverse::<TcpStream, H>();
    //     }
    // }
}

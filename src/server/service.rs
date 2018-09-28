use std::marker::PhantomData;

use actix_net::service::{NewService, Service};
use futures::future::{ok, FutureResult};
use futures::{Async, Poll};

use super::channel::HttpChannel;
use super::handler::HttpHandler;
use super::settings::WorkerSettings;
use super::IoStream;

pub(crate) struct HttpService<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    settings: WorkerSettings<H>,
    _t: PhantomData<Io>,
}

impl<H, Io> HttpService<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    pub fn new(settings: WorkerSettings<H>) -> Self {
        HttpService {
            settings,
            _t: PhantomData,
        }
    }
}

impl<H, Io> NewService for HttpService<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    type Request = Io;
    type Response = ();
    type Error = ();
    type InitError = ();
    type Service = HttpServiceHandler<H, Io>;
    type Future = FutureResult<Self::Service, Self::Error>;

    fn new_service(&self) -> Self::Future {
        ok(HttpServiceHandler::new(self.settings.clone()))
    }
}

pub(crate) struct HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    settings: WorkerSettings<H>,
    // tcp_ka: Option<Duration>,
    _t: PhantomData<Io>,
}

impl<H, Io> HttpServiceHandler<H, Io>
where
    H: HttpHandler,
    Io: IoStream,
{
    fn new(settings: WorkerSettings<H>) -> HttpServiceHandler<H, Io> {
        // let tcp_ka = if let KeepAlive::Tcp(val) = keep_alive {
        //     Some(Duration::new(val as u64, 0))
        // } else {
        //     None
        // };

        HttpServiceHandler {
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
}

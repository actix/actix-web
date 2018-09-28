//! Support for `Stream<Item=T::AsyncReady+AsyncWrite>`, deprecated!
use std::{io, net};

use actix::{Actor, Arbiter, AsyncContext, Context, Handler, Message};
use futures::Stream;
use tokio_io::{AsyncRead, AsyncWrite};

use super::channel::{HttpChannel, WrapperStream};
use super::handler::{HttpHandler, IntoHttpHandler};
use super::http::HttpServer;
use super::settings::{ServerSettings, WorkerSettings};

impl<T: AsyncRead + AsyncWrite + 'static> Message for WrapperStream<T> {
    type Result = ();
}

impl<H, F> HttpServer<H, F>
where
    H: IntoHttpHandler,
    F: Fn() -> H + Send + Clone,
{
    #[doc(hidden)]
    #[deprecated(since = "0.7.8")]
    /// Start listening for incoming connections from a stream.
    ///
    /// This method uses only one thread for handling incoming connections.
    pub fn start_incoming<T, S>(self, stream: S, secure: bool)
    where
        S: Stream<Item = T, Error = io::Error> + 'static,
        T: AsyncRead + AsyncWrite + 'static,
    {
        // set server settings
        let addr: net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let apps = (self.factory)().into_handler();
        let settings = WorkerSettings::new(
            apps,
            self.keep_alive,
            self.client_timeout as u64,
            ServerSettings::new(Some(addr), &self.host, secure),
        );

        // start server
        HttpIncoming::create(move |ctx| {
            ctx.add_message_stream(
                stream.map_err(|_| ()).map(move |t| WrapperStream::new(t)),
            );
            HttpIncoming { settings }
        });
    }
}

struct HttpIncoming<H: HttpHandler> {
    settings: WorkerSettings<H>,
}

impl<H: HttpHandler> Actor for HttpIncoming<H> {
    type Context = Context<Self>;
}

impl<T, H> Handler<WrapperStream<T>> for HttpIncoming<H>
where
    T: AsyncRead + AsyncWrite,
    H: HttpHandler,
{
    type Result = ();

    fn handle(&mut self, msg: WrapperStream<T>, _: &mut Context<Self>) -> Self::Result {
        Arbiter::spawn(HttpChannel::new(self.settings.clone(), msg, None));
    }
}

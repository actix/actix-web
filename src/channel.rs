use std::net::SocketAddr;

use actix::dev::*;
use bytes::Bytes;
use futures::{Future, Poll, Async};
use tokio_io::{AsyncRead, AsyncWrite};

use h1;
use h2;
use pipeline::Pipeline;
use httprequest::HttpRequest;
use server::ServerSettings;

/// Low level http request handler
pub trait HttpHandler: 'static {
    /// Handle request
    fn handle(&self, req: HttpRequest) -> Result<Pipeline, HttpRequest>;
}

/// Conversion helper trait
pub trait IntoHttpHandler {
    /// The associated type which is result of conversion.
    type Handler: HttpHandler;

    /// Convert into `HttpHandler` object.
    fn into_handler(self) -> Self::Handler;
}

impl<T: HttpHandler> IntoHttpHandler for T {
    type Handler = T;

    fn into_handler(self) -> Self::Handler {
        self
    }
}

enum HttpProtocol<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: 'static
{
    H1(h1::Http1<T, H>),
    H2(h2::Http2<T, H>),
}

#[doc(hidden)]
pub struct HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: 'static
{
    proto: Option<HttpProtocol<T, H>>,
}

impl<T, H> HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    pub fn new(settings: ServerSettings<H>,
               stream: T, peer: Option<SocketAddr>, http2: bool) -> HttpChannel<T, H>
    {
        if http2 {
            HttpChannel {
                proto: Some(HttpProtocol::H2(
                    h2::Http2::new(settings, stream, peer, Bytes::new()))) }
        } else {
            HttpChannel {
                proto: Some(HttpProtocol::H1(
                    h1::Http1::new(settings, stream, peer))) }
        }
    }
}

/*impl<T: 'static, A: 'static, H: 'static> Drop for HttpChannel<T, A, H> {
    fn drop(&mut self) {
        println!("Drop http channel");
    }
}*/

impl<T, H> Actor for HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    type Context = Context<Self>;
}

impl<T, H> Future for HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                match h1.poll() {
                    Ok(Async::Ready(h1::Http1Result::Done)) =>
                        return Ok(Async::Ready(())),
                    Ok(Async::Ready(h1::Http1Result::Switch)) => (),
                    Ok(Async::NotReady) =>
                        return Ok(Async::NotReady),
                    Err(_) =>
                        return Err(()),
                }
            }
            Some(HttpProtocol::H2(ref mut h2)) =>
                return h2.poll(),
            None => unreachable!(),
        }

        // upgrade to h2
        let proto = self.proto.take().unwrap();
        match proto {
            HttpProtocol::H1(h1) => {
                let (settings, stream, addr, buf) = h1.into_inner();
                self.proto = Some(
                    HttpProtocol::H2(h2::Http2::new(settings, stream, addr, buf)));
                self.poll()
            }
            _ => unreachable!()
        }
    }
}

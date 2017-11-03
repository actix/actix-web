use std::{io, net, mem};
use std::rc::Rc;
use std::marker::PhantomData;

use actix::dev::*;
use futures::{Future, Poll, Async, Stream};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_io::{AsyncRead, AsyncWrite};

#[cfg(feature="tls")]
use native_tls::TlsAcceptor;
#[cfg(feature="tls")]
use tokio_tls::{TlsStream, TlsAcceptorExt};

use h1;
use h2;
use task::Task;
use payload::Payload;
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

    fn bind<S: net::ToSocketAddrs>(&self, addr: S)
                                   -> io::Result<Vec<(net::SocketAddr, TcpListener)>>
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
            Ok(addrs)
        }
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
        let addrs = self.bind(addr)?;

        Ok(HttpServer::create(move |ctx| {
            for (addr, tcp) in addrs {
                info!("Starting http server on {}", addr);
                ctx.add_stream(tcp.incoming().map(|(t, a)| IoStream(t, a)));
            }
            self
        }))
    }
}

#[cfg(feature="tls")]
impl<H: HttpHandler> HttpServer<TlsStream<TcpStream>, net::SocketAddr, H> {

    /// Start listening for incomming tls connections.
    ///
    /// This methods converts address to list of `SocketAddr`
    /// then binds to all available addresses.
    pub fn serve_tls<S, Addr>(self, addr: S, pkcs12: ::Pkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;
        let acceptor = match TlsAcceptor::builder(pkcs12) {
            Ok(builder) => {
                match builder.build() {
                    Ok(acceptor) => Rc::new(acceptor),
                    Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
                }
            }
            Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
        };

        Ok(HttpServer::create(move |ctx| {
            for (addr, tcp) in addrs {
                info!("Starting tls http server on {}", addr);

                let acc = acceptor.clone();
                ctx.add_stream(tcp.incoming().and_then(move |(stream, addr)| {
                    println!("SSL");
                    TlsAcceptorExt::accept_async(acc.as_ref(), stream)
                        .map(move |t| {
                            IoStream(t, addr)
                        })
                        .map_err(|err| {
                            trace!("Error during handling tls connection: {}", err);
                            io::Error::new(io::ErrorKind::Other, err)
                        })
                }));
            }
            self
        }))
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
    fn error(&mut self, err: io::Error, _: &mut Context<Self>) {
        println!("Error handling request: {}", err)
    }

    fn handle(&mut self, msg: IoStream<T, A>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T, A>>
    {
        Arbiter::handle().spawn(
            HttpChannel{
                proto: Protocol::H1(h1::Http1::new(msg.0, msg.1, Rc::clone(&self.h)))
            });
        Self::empty()
    }
}

enum Protocol<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static, A: 'static, H: 'static
{
    H1(h1::Http1<T, A, H>),
    H2(h2::Http2<T, A, H>),
    None,
}

pub struct HttpChannel<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static, A: 'static, H: 'static
{
    proto: Protocol<T, A, H>,
}

/*impl<T: 'static, A: 'static, H: 'static> Drop for HttpChannel<T, A, H> {
    fn drop(&mut self) {
        println!("Drop http channel");
    }
}*/

impl<T, A, H> Actor for HttpChannel<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static, A: 'static, H: HttpHandler + 'static
{
    type Context = Context<Self>;
}

impl<T, A, H> Future for HttpChannel<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static, A: 'static, H: HttpHandler + 'static
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.proto {
            Protocol::H1(ref mut h1) => {
                match h1.poll() {
                    Ok(Async::Ready(h1::Http1Result::Done)) =>
                        return Ok(Async::Ready(())),
                    Ok(Async::Ready(h1::Http1Result::Upgrade)) => (),
                    Ok(Async::NotReady) =>
                        return Ok(Async::NotReady),
                    Err(_) =>
                        return Err(()),
                }
            }
            Protocol::H2(ref mut h2) =>
                return h2.poll(),
            Protocol::None =>
                unreachable!()
        }

        // upgrade to h2
        let proto = mem::replace(&mut self.proto, Protocol::None);
        match proto {
            Protocol::H1(h1) => {
                let (stream, addr, router, buf) = h1.into_inner();
                self.proto = Protocol::H2(h2::Http2::new(stream, addr, router, buf));
                return self.poll()
            }
            _ => unreachable!()
        }
    }
}

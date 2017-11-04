use std::{io, net};
use std::rc::Rc;
use std::marker::PhantomData;

use actix::dev::*;
use futures::Stream;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::net::{TcpListener, TcpStream};

#[cfg(feature="tls")]
use native_tls::TlsAcceptor;
#[cfg(feature="tls")]
use tokio_tls::{TlsStream, TlsAcceptorExt};

#[cfg(feature="alpn")]
use openssl::ssl::{SslMethod, SslAcceptorBuilder};
#[cfg(feature="alpn")]
use openssl::pkcs12::ParsedPkcs12;
#[cfg(feature="alpn")]
use tokio_openssl::{SslStream, SslAcceptorExt};

use channel::{HttpChannel, HttpHandler};


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
            ctx.add_stream(stream.map(|(t, a)| IoStream(t, a, false)));
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
                ctx.add_stream(tcp.incoming().map(|(t, a)| IoStream(t, a, false)));
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

#[cfg(feature="alpn")]
impl<H: HttpHandler> HttpServer<SslStream<TcpStream>, net::SocketAddr, H> {

    /// Start listening for incomming tls connections.
    ///
    /// This methods converts address to list of `SocketAddr`
    /// then binds to all available addresses.
    pub fn serve_tls<S, Addr>(self, addr: S, identity: ParsedPkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;
        let acceptor = match SslAcceptorBuilder::mozilla_intermediate(SslMethod::tls(),
                                                                      &identity.pkey,
                                                                      &identity.cert,
                                                                      &identity.chain)
        {
            Ok(mut builder) => {
                match builder.builder_mut().set_alpn_protocols(&[b"h2", b"http/1.1"]) {
                    Ok(_) => builder.build(),
                    Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err)),
                }
            },
            Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
        };

        Ok(HttpServer::create(move |ctx| {
            for (addr, tcp) in addrs {
                info!("Starting tls http server on {}", addr);

                let acc = acceptor.clone();
                ctx.add_stream(tcp.incoming().and_then(move |(stream, addr)| {
                    SslAcceptorExt::accept_async(&acc, stream)
                        .map(move |stream| {
                            let http2 = if let Some(p) =
                                stream.get_ref().ssl().selected_alpn_protocol()
                            {
                                p.len() == 2 && &p == b"h2"
                            } else {
                                false
                            };
                            IoStream(stream, addr, http2)
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

struct IoStream<T, A>(T, A, bool);

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
        debug!("Error handling request: {}", err)
    }

    fn handle(&mut self, msg: IoStream<T, A>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T, A>>
    {
        Arbiter::handle().spawn(
            HttpChannel::new(msg.0, msg.1, Rc::clone(&self.h), msg.2));
        Self::empty()
    }
}

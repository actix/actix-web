use std::{io, net};
use std::rc::Rc;
use std::net::SocketAddr;
use std::marker::PhantomData;

use actix::dev::*;
use futures::Stream;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::net::{TcpListener, TcpStream};

#[cfg(feature="tls")]
use futures::Future;
#[cfg(feature="tls")]
use native_tls::TlsAcceptor;
#[cfg(feature="tls")]
use tokio_tls::{TlsStream, TlsAcceptorExt};

#[cfg(feature="alpn")]
use futures::Future;
#[cfg(feature="alpn")]
use openssl::ssl::{SslMethod, SslAcceptorBuilder};
#[cfg(feature="alpn")]
use openssl::pkcs12::ParsedPkcs12;
#[cfg(feature="alpn")]
use tokio_openssl::{SslStream, SslAcceptorExt};

use channel::{HttpChannel, HttpHandler, IntoHttpHandler};

/// Various server settings
#[derive(Debug, Clone)]
pub struct ServerSettings {
    addr: Option<SocketAddr>,
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
    fn new(addr: Option<SocketAddr>, secure: bool) -> Self {
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
    pub fn local_addr(&self) -> Option<SocketAddr> {
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
    pub fn new<V, U: IntoIterator<Item=V>>(handler: U) -> Self
        where V: IntoHttpHandler<Handler=H>
    {
        let apps: Vec<_> = handler.into_iter().map(|h| h.into_handler()).collect();

        HttpServer{ h: Rc::new(apps),
                    io: PhantomData,
                    addr: PhantomData }
    }
}

impl<T, A, H> HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static,
          H: HttpHandler,
{
    /// Start listening for incomming connections from stream.
    pub fn serve_incoming<S, Addr>(mut self, stream: S, secure: bool) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: Stream<Item=(T, A), Error=io::Error> + 'static
    {
        // set server settings
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let settings = ServerSettings::new(Some(addr), secure);
        for h in Rc::get_mut(&mut self.h).unwrap().iter_mut() {
            h.server_settings(settings.clone());
        }

        // start server
        Ok(HttpServer::create(move |ctx| {
            ctx.add_stream(stream.map(
                move |(t, _)| IoStream{io: t, peer: None, http2: false}));
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
    pub fn serve<S, Addr>(mut self, addr: S) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;

        // set server settings
        let settings = ServerSettings::new(Some(addrs[0].0), false);
        for h in Rc::get_mut(&mut self.h).unwrap().iter_mut() {
            h.server_settings(settings.clone());
        }

        // start server
        Ok(HttpServer::create(move |ctx| {
            for (addr, tcp) in addrs {
                info!("Starting http server on {}", addr);

                ctx.add_stream(tcp.incoming().map(
                    move |(t, a)| IoStream{io: t, peer: Some(a), http2: false}));
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
    pub fn serve_tls<S, Addr>(mut self, addr: S, pkcs12: ::Pkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;

        // set server settings
        let settings = ServerSettings::new(Some(addrs[0].0), true);
        for h in Rc::get_mut(&mut self.h).unwrap().iter_mut() {
            h.server_settings(settings.clone());
        }

        let acceptor = match TlsAcceptor::builder(pkcs12) {
            Ok(builder) => {
                match builder.build() {
                    Ok(acceptor) => Rc::new(acceptor),
                    Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
                }
            }
            Err(err) => return Err(io::Error::new(io::ErrorKind::Other, err))
        };

        // start server
        Ok(HttpServer::create(move |ctx| {
            for (srv, tcp) in addrs {
                info!("Starting tls http server on {}", srv);

                let acc = acceptor.clone();
                ctx.add_stream(tcp.incoming().and_then(move |(stream, addr)| {
                    TlsAcceptorExt::accept_async(acc.as_ref(), stream)
                        .map(move |t| IoStream{io: t, peer: Some(addr), http2: false})
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
    pub fn serve_tls<S, Addr>(mut self, addr: S, identity: ParsedPkcs12) -> io::Result<Addr>
        where Self: ActorAddress<Self, Addr>,
              S: net::ToSocketAddrs,
    {
        let addrs = self.bind(addr)?;

        // set server settings
        let settings = ServerSettings::new(Some(addrs[0].0), true);
        for h in Rc::get_mut(&mut self.h).unwrap().iter_mut() {
            h.server_settings(settings.clone());
        }

        let acceptor = match SslAcceptorBuilder::mozilla_intermediate(
            SslMethod::tls(), &identity.pkey, &identity.cert, &identity.chain)
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
            for (srv, tcp) in addrs {
                info!("Starting tls http server on {}", srv);

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
                            IoStream{io: stream, peer: Some(addr), http2: http2}
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

struct IoStream<T> {
    io: T,
    peer: Option<SocketAddr>,
    http2: bool,
}

impl<T> ResponseType for IoStream<T>
    where T: AsyncRead + AsyncWrite + 'static
{
    type Item = ();
    type Error = ();
}

impl<T, A, H> StreamHandler<IoStream<T>, io::Error> for HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          A: 'static {}

impl<T, A, H> Handler<IoStream<T>, io::Error> for HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          A: 'static,
{
    fn error(&mut self, err: io::Error, _: &mut Context<Self>) {
        debug!("Error handling request: {}", err)
    }

    fn handle(&mut self, msg: IoStream<T>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T>>
    {
        Arbiter::handle().spawn(
            HttpChannel::new(Rc::clone(&self.h), msg.io, msg.peer, msg.http2));
        Self::empty()
    }
}

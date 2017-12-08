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
pub struct ServerSettings<H> (Rc<InnerServerSettings<H>>);

struct InnerServerSettings<H> {
    h: Vec<H>,
    addr: Option<SocketAddr>,
    secure: bool,
    sethost: bool,
}

impl<H> Clone for ServerSettings<H> {
    fn clone(&self) -> Self {
        ServerSettings(Rc::clone(&self.0))
    }
}

impl<H> ServerSettings<H> {
    /// Crate server settings instance
    fn new(h: Vec<H>, addr: Option<SocketAddr>, secure: bool, sethost: bool) -> Self {
        ServerSettings(
            Rc::new(InnerServerSettings {
                h: h,
                addr: addr,
                secure: secure,
                sethost: sethost }))
    }

    /// Returns list of http handlers
    pub fn handlers(&self) -> &Vec<H> {
        &self.0.h
    }
    /// Returns the socket address of the local half of this TCP connection
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.0.addr
    }

    /// Returns true if connection is secure(https)
    pub fn secure(&self) -> bool {
        self.0.secure
    }

    /// Should http channel set *HOST* header
    pub fn set_host_header(&self) -> bool {
        self.0.sethost
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
    h: Option<Vec<H>>,
    io: PhantomData<T>,
    addr: PhantomData<A>,
    sethost: bool,
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

        HttpServer {h: Some(apps),
                    io: PhantomData,
                    addr: PhantomData,
                    sethost: false}
    }

    /// Set *HOST* header if not set
    pub fn set_host_header(mut self) -> Self {
        self.sethost = true;
        self
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
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let settings = ServerSettings::new(
            self.h.take().unwrap(), Some(addr), secure, self.sethost);

        Ok(HttpServer::create(move |ctx| {
            ctx.add_stream(stream.map(
                move |(t, _)| IoStream{settings: settings.clone(),
                                       io: t, peer: None, http2: false}));
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
        let settings = ServerSettings::new(
            self.h.take().unwrap(), Some(addrs[0].0), false, self.sethost);

        Ok(HttpServer::create(move |ctx| {
            for (addr, tcp) in addrs {
                info!("Starting http server on {}", addr);
                let s = settings.clone();
                ctx.add_stream(tcp.incoming().map(
                    move |(t, a)| IoStream{settings: s.clone(),
                                           io: t, peer: Some(a), http2: false}));
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
        let settings = ServerSettings::new(
            self.h.take().unwrap(), Some(addrs[0].0.clone()), true, self.sethost);

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
            for (srv, tcp) in addrs {
                info!("Starting tls http server on {}", srv);

                let st = settings.clone();
                let acc = acceptor.clone();
                ctx.add_stream(tcp.incoming().and_then(move |(stream, addr)| {
                    let st2 = st.clone();
                    TlsAcceptorExt::accept_async(acc.as_ref(), stream)
                        .map(move |t|
                             IoStream{settings: st2.clone(),
                                      io: t, peer: Some(addr), http2: false})
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
        let settings = ServerSettings::new(
            self.h.take().unwrap(), Some(addrs[0].0.clone()), true, self.sethost);

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

                let st = settings.clone();
                let acc = acceptor.clone();
                ctx.add_stream(tcp.incoming().and_then(move |(stream, addr)| {
                    let st2 = st.clone();
                    SslAcceptorExt::accept_async(&acc, stream)
                        .map(move |stream| {
                            let http2 = if let Some(p) =
                                stream.get_ref().ssl().selected_alpn_protocol()
                            {
                                p.len() == 2 && &p == b"h2"
                            } else {
                                false
                            };
                            IoStream{settings: st2.clone(),
                                     io: stream, peer: Some(addr), http2: http2}
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

struct IoStream<T, H> {
    io: T,
    peer: Option<SocketAddr>,
    http2: bool,
    settings: ServerSettings<H>,
}

impl<T, H> ResponseType for IoStream<T, H>
    where T: AsyncRead + AsyncWrite + 'static
{
    type Item = ();
    type Error = ();
}

impl<T, A, H> StreamHandler<IoStream<T, H>, io::Error> for HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          A: 'static {}

impl<T, A, H> Handler<IoStream<T, H>, io::Error> for HttpServer<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          H: HttpHandler + 'static,
          A: 'static,
{
    fn error(&mut self, err: io::Error, _: &mut Context<Self>) {
        debug!("Error handling request: {}", err)
    }

    fn handle(&mut self, msg: IoStream<T, H>, _: &mut Context<Self>)
              -> Response<Self, IoStream<T, H>>
    {
        Arbiter::handle().spawn(
            HttpChannel::new(msg.settings, msg.io, msg.peer, msg.http2));
        Self::empty()
    }
}

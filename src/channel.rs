use std::{ptr, mem, time};
use std::rc::Rc;
use std::net::{SocketAddr, Shutdown};

use bytes::Bytes;
use futures::{Future, Poll, Async};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_core::net::TcpStream;

use {h1, h2};
use error::Error;
use h1writer::Writer;
use httprequest::HttpRequest;
use server::ServerSettings;
use worker::WorkerSettings;

/// Low level http request handler
#[allow(unused_variables)]
pub trait HttpHandler: 'static {

    /// Handle request
    fn handle(&mut self, req: HttpRequest) -> Result<Box<HttpHandlerTask>, HttpRequest>;
}

pub trait HttpHandlerTask {

    fn poll_io(&mut self, io: &mut Writer) -> Poll<bool, Error>;

    fn poll(&mut self) -> Poll<(), Error>;

    fn disconnected(&mut self);
}

/// Conversion helper trait
pub trait IntoHttpHandler {
    /// The associated type which is result of conversion.
    type Handler: HttpHandler;

    /// Convert into `HttpHandler` object.
    fn into_handler(self, settings: ServerSettings) -> Self::Handler;
}

impl<T: HttpHandler> IntoHttpHandler for T {
    type Handler = T;

    fn into_handler(self, _: ServerSettings) -> Self::Handler {
        self
    }
}

enum HttpProtocol<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    H1(h1::Http1<T, H>),
    H2(h2::Http2<T, H>),
}

#[doc(hidden)]
pub struct HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    proto: Option<HttpProtocol<T, H>>,
    node: Option<Node<HttpChannel<T, H>>>,
}

impl<T, H> Drop for HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    fn drop(&mut self) {
        self.shutdown()
    }
}

impl<T, H> HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    pub(crate) fn new(h: Rc<WorkerSettings<H>>,
                      io: T, peer: Option<SocketAddr>, http2: bool) -> HttpChannel<T, H>
    {
        h.add_channel();
        if http2 {
            HttpChannel {
                node: None,
                proto: Some(HttpProtocol::H2(
                    h2::Http2::new(h, io, peer, Bytes::new()))) }
        } else {
            HttpChannel {
                node: None,
                proto: Some(HttpProtocol::H1(
                    h1::Http1::new(h, io, peer))) }
        }
    }

    fn io(&mut self) -> Option<&mut T> {
        match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                Some(h1.io())
            }
            _ => None,
        }
    }

    fn shutdown(&mut self) {
        match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                let _ = h1.io().shutdown();
            }
            Some(HttpProtocol::H2(ref mut h2)) => {
                h2.shutdown()
            }
            _ => unreachable!(),
        }
    }
}

/*impl<T, H> Drop for HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    fn drop(&mut self) {
        println!("Drop http channel");
    }
}*/

impl<T, H> Future for HttpChannel<T, H>
    where T: AsyncRead + AsyncWrite + 'static, H: HttpHandler + 'static
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.node.is_none() {
            self.node = Some(Node::new(self));
            match self.proto {
                Some(HttpProtocol::H1(ref mut h1)) => {
                    h1.settings().head().insert(self.node.as_ref().unwrap());
                }
                Some(HttpProtocol::H2(ref mut h2)) => {
                    h2.settings().head().insert(self.node.as_ref().unwrap());
                }
                _ => unreachable!(),
            }
        }

        match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                match h1.poll() {
                    Ok(Async::Ready(h1::Http1Result::Done)) => {
                        h1.settings().remove_channel();
                        self.node.as_ref().unwrap().remove();
                        return Ok(Async::Ready(()))
                    }
                    Ok(Async::Ready(h1::Http1Result::Switch)) => (),
                    Ok(Async::NotReady) =>
                        return Ok(Async::NotReady),
                    Err(_) => {
                        h1.settings().remove_channel();
                        self.node.as_ref().unwrap().remove();
                        return Err(())
                    }
                }
            }
            Some(HttpProtocol::H2(ref mut h2)) => {
                let result = h2.poll();
                match result {
                    Ok(Async::Ready(())) | Err(_) => {
                        h2.settings().remove_channel();
                        self.node.as_ref().unwrap().remove();
                    }
                    _ => (),
                }
                return result
            }
            None => unreachable!(),
        }

        // upgrade to h2
        let proto = self.proto.take().unwrap();
        match proto {
            HttpProtocol::H1(h1) => {
                let (h, io, addr, buf) = h1.into_inner();
                self.proto = Some(
                    HttpProtocol::H2(h2::Http2::new(h, io, addr, buf)));
                self.poll()
            }
            _ => unreachable!()
        }
    }
}

pub(crate) struct Node<T>
{
    next: Option<*mut Node<()>>,
    prev: Option<*mut Node<()>>,
    element: *mut T,
}

impl<T> Node<T>
{
    fn new(el: &mut T) -> Self {
        Node {
            next: None,
            prev: None,
            element: el as *mut _,
        }
    }

    fn insert<I>(&self, next: &Node<I>) {
        #[allow(mutable_transmutes)]
        unsafe {
            if let Some(ref next2) = self.next {
                let n: &mut Node<()> = mem::transmute(next2.as_ref().unwrap());
                n.prev = Some(next as *const _ as *mut _);
            }
            let slf: &mut Node<T> = mem::transmute(self);
            slf.next = Some(next as *const _ as *mut _);

            let next: &mut Node<T> = mem::transmute(next);
            next.prev = Some(slf as *const _ as *mut _);
        }
    }

    fn remove(&self) {
        #[allow(mutable_transmutes)]
        unsafe {
            if let Some(ref prev) = self.prev {
                let p: &mut Node<()> = mem::transmute(prev.as_ref().unwrap());
                let slf: &mut Node<T> = mem::transmute(self);
                p.next = slf.next.take();
            }
        }
    }
}


impl Node<()> {

    pub(crate) fn head() -> Self {
        Node {
            next: None,
            prev: None,
            element: ptr::null_mut(),
        }
    }

    pub(crate) fn traverse<H>(&self) where H: HttpHandler + 'static {
        let mut next = self.next.as_ref();
        loop {
            if let Some(n) = next {
                unsafe {
                    let n: &Node<()> = mem::transmute(n.as_ref().unwrap());
                    next = n.next.as_ref();

                    if !n.element.is_null() {
                        let ch: &mut HttpChannel<TcpStream, H> = mem::transmute(
                            &mut *(n.element as *mut _));
                        if let Some(io) = ch.io() {
                            let _ = TcpStream::set_linger(io, Some(time::Duration::new(0, 0)));
                            let _ = TcpStream::shutdown(io, Shutdown::Both);
                            continue;
                        }
                        ch.shutdown();
                    }
                }
            } else {
                return
            }
        }
    }
}

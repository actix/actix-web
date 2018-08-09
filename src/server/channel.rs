use std::net::{Shutdown, SocketAddr};
use std::rc::Rc;
use std::{io, ptr, time};

use bytes::{Buf, BufMut, BytesMut};
use futures::{Async, Future, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

use super::settings::WorkerSettings;
use super::{h1, h2, ConnectionTag, HttpHandler, IoStream};

const HTTP2_PREFACE: [u8; 14] = *b"PRI * HTTP/2.0";

enum HttpProtocol<T: IoStream, H: HttpHandler + 'static> {
    H1(h1::Http1<T, H>),
    H2(h2::Http2<T, H>),
    Unknown(Rc<WorkerSettings<H>>, Option<SocketAddr>, T, BytesMut),
}

enum ProtocolKind {
    Http1,
    Http2,
}

#[doc(hidden)]
pub struct HttpChannel<T, H>
where
    T: IoStream,
    H: HttpHandler + 'static,
{
    proto: Option<HttpProtocol<T, H>>,
    node: Option<Node<HttpChannel<T, H>>>,
    _tag: ConnectionTag,
}

impl<T, H> HttpChannel<T, H>
where
    T: IoStream,
    H: HttpHandler + 'static,
{
    pub(crate) fn new(
        settings: Rc<WorkerSettings<H>>, io: T, peer: Option<SocketAddr>,
    ) -> HttpChannel<T, H> {
        let _tag = settings.connection();

        HttpChannel {
            _tag,
            node: None,
            proto: Some(HttpProtocol::Unknown(
                settings,
                peer,
                io,
                BytesMut::with_capacity(8192),
            )),
        }
    }

    fn shutdown(&mut self) {
        match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                let io = h1.io();
                let _ = IoStream::set_linger(io, Some(time::Duration::new(0, 0)));
                let _ = IoStream::shutdown(io, Shutdown::Both);
            }
            Some(HttpProtocol::H2(ref mut h2)) => h2.shutdown(),
            _ => (),
        }
    }
}

impl<T, H> Future for HttpChannel<T, H>
where
    T: IoStream,
    H: HttpHandler + 'static,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.node.is_some() {
            let el = self as *mut _;
            self.node = Some(Node::new(el));
            let _ = match self.proto {
                Some(HttpProtocol::H1(ref mut h1)) => {
                    self.node.as_mut().map(|n| h1.settings().head().insert(n))
                }
                Some(HttpProtocol::H2(ref mut h2)) => {
                    self.node.as_mut().map(|n| h2.settings().head().insert(n))
                }
                Some(HttpProtocol::Unknown(ref mut settings, _, _, _)) => {
                    self.node.as_mut().map(|n| settings.head().insert(n))
                }
                None => unreachable!(),
            };
        }

        let kind = match self.proto {
            Some(HttpProtocol::H1(ref mut h1)) => {
                let result = h1.poll();
                match result {
                    Ok(Async::Ready(())) | Err(_) => {
                        if let Some(n) = self.node.as_mut() {
                            n.remove()
                        };
                    }
                    _ => (),
                }
                return result;
            }
            Some(HttpProtocol::H2(ref mut h2)) => {
                let result = h2.poll();
                match result {
                    Ok(Async::Ready(())) | Err(_) => {
                        if let Some(n) = self.node.as_mut() {
                            n.remove()
                        };
                    }
                    _ => (),
                }
                return result;
            }
            Some(HttpProtocol::Unknown(_, _, ref mut io, ref mut buf)) => {
                match io.read_available(buf) {
                    Ok(Async::Ready(true)) | Err(_) => {
                        debug!("Ignored premature client disconnection");
                        if let Some(n) = self.node.as_mut() {
                            n.remove()
                        };
                        return Err(());
                    }
                    _ => (),
                }

                if buf.len() >= 14 {
                    if buf[..14] == HTTP2_PREFACE[..] {
                        ProtocolKind::Http2
                    } else {
                        ProtocolKind::Http1
                    }
                } else {
                    return Ok(Async::NotReady);
                }
            }
            None => unreachable!(),
        };

        // upgrade to specific http protocol
        if let Some(HttpProtocol::Unknown(settings, addr, io, buf)) = self.proto.take() {
            match kind {
                ProtocolKind::Http1 => {
                    self.proto =
                        Some(HttpProtocol::H1(h1::Http1::new(settings, io, addr, buf)));
                    return self.poll();
                }
                ProtocolKind::Http2 => {
                    self.proto = Some(HttpProtocol::H2(h2::Http2::new(
                        settings,
                        io,
                        addr,
                        buf.freeze(),
                    )));
                    return self.poll();
                }
            }
        }
        unreachable!()
    }
}

pub(crate) struct Node<T> {
    next: Option<*mut Node<T>>,
    prev: Option<*mut Node<T>>,
    element: *mut T,
}

impl<T> Node<T> {
    fn new(el: *mut T) -> Self {
        Node {
            next: None,
            prev: None,
            element: el,
        }
    }

    fn insert<I>(&mut self, next: &mut Node<I>) {
        unsafe {
            let next: *mut Node<T> = next as *const _ as *mut _;

            if let Some(ref mut next2) = self.next {
                let n = next2.as_mut().unwrap();
                n.prev = Some(next);
            }
            self.next = Some(next);

            let next: &mut Node<T> = &mut *next;
            next.prev = Some(self as *mut _);
        }
    }

    fn remove(&mut self) {
        unsafe {
            self.element = ptr::null_mut();
            let next = self.next.take();
            let mut prev = self.prev.take();

            if let Some(ref mut prev) = prev {
                prev.as_mut().unwrap().next = next;
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

    pub(crate) fn traverse<T, H>(&self)
    where
        T: IoStream,
        H: HttpHandler + 'static,
    {
        let mut next = self.next.as_ref();
        loop {
            if let Some(n) = next {
                unsafe {
                    let n: &Node<()> = &*(n.as_ref().unwrap() as *const _);
                    next = n.next.as_ref();

                    if !n.element.is_null() {
                        let ch: &mut HttpChannel<T, H> =
                            &mut *(&mut *(n.element as *mut _) as *mut () as *mut _);
                        ch.shutdown();
                    }
                }
            } else {
                return;
            }
        }
    }
}

/// Wrapper for `AsyncRead + AsyncWrite` types
pub(crate) struct WrapperStream<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    io: T,
}

impl<T> WrapperStream<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    pub fn new(io: T) -> Self {
        WrapperStream { io }
    }
}

impl<T> IoStream for WrapperStream<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    #[inline]
    fn shutdown(&mut self, _: Shutdown) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn set_nodelay(&mut self, _: bool) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn set_linger(&mut self, _: Option<time::Duration>) -> io::Result<()> {
        Ok(())
    }
}

impl<T> io::Read for WrapperStream<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io.read(buf)
    }
}

impl<T> io::Write for WrapperStream<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io.write(buf)
    }
    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.io.flush()
    }
}

impl<T> AsyncRead for WrapperStream<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    #[inline]
    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.io.read_buf(buf)
    }
}

impl<T> AsyncWrite for WrapperStream<T>
where
    T: AsyncRead + AsyncWrite + 'static,
{
    #[inline]
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.io.shutdown()
    }
    #[inline]
    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.io.write_buf(buf)
    }
}

use std::{io, cmp, mem};
use std::rc::Rc;
use std::io::{Read, Write};
use std::cell::UnsafeCell;
use std::collections::VecDeque;

use http::request::Parts;
use http2::{RecvStream};
use http2::server::{Server, Handshake, Respond};
use bytes::{Buf, Bytes};
use futures::{Async, Poll, Future, Stream};
use tokio_io::{AsyncRead, AsyncWrite};

use task::Task;
use server::HttpHandler;
use httpcodes::HTTPNotFound;
use httprequest::HttpRequest;
use payload::{Payload, PayloadError, PayloadSender};


pub(crate) struct Http2<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static, A: 'static, H: 'static
{
    router: Rc<Vec<H>>,
    #[allow(dead_code)]
    addr: A,
    state: State<IoWrapper<T>>,
    error: bool,
    tasks: VecDeque<Entry>,
}

enum State<T: AsyncRead + AsyncWrite> {
    Handshake(Handshake<T, Bytes>),
    Server(Server<T, Bytes>),
    Empty,
}

impl<T, A, H> Http2<T, A, H>
    where T: AsyncRead + AsyncWrite + 'static,
          A: 'static,
          H: HttpHandler + 'static
{
    pub fn new(stream: T, addr: A, router: Rc<Vec<H>>, buf: Bytes) -> Self {
        Http2{ router: router,
               addr: addr,
               error: false,
               tasks: VecDeque::new(),
               state: State::Handshake(
                   Server::handshake(IoWrapper{unread: Some(buf), inner: stream})) }
    }

    pub fn poll(&mut self) -> Poll<(), ()> {
        // handshake
        self.state = if let State::Handshake(ref mut handshake) = self.state {
            match handshake.poll() {
                Ok(Async::Ready(srv)) => {
                    State::Server(srv)
                },
                Ok(Async::NotReady) =>
                    return Ok(Async::NotReady),
                Err(err) => {
                    trace!("Error handling connection: {}", err);
                    return Err(())
                }
            }
        } else {
            mem::replace(&mut self.state, State::Empty)
        };

        // get request
        let poll = if let State::Server(ref mut server) = self.state {
            server.poll()
        } else {
            unreachable!("Http2::poll() state was not advanced completely!")
        };

        match poll {
            Ok(Async::NotReady) => {
                // Ok(Async::NotReady);
                ()
            }
            Err(err) => {
                trace!("Connection error: {}", err);
                self.error = true;
            },
            Ok(Async::Ready(None)) => {

            },
            Ok(Async::Ready(Some((req, resp)))) => {
                let (parts, body) = req.into_parts();
                let entry = Entry::new(parts, body, resp, &self.router);
            }
        }

        Ok(Async::Ready(()))
    }
}

struct Entry {
    task: Task,
    req: UnsafeCell<HttpRequest>,
    payload: PayloadSender,
    recv: RecvStream,
    respond: Respond<Bytes>,
    eof: bool,
    error: bool,
    finished: bool,
}

impl Entry {
    fn new<H>(parts: Parts,
              recv: RecvStream,
              resp: Respond<Bytes>,
              router: &Rc<Vec<H>>) -> Entry
        where H: HttpHandler + 'static
    {
        let path = parts.uri.path().to_owned();
        let query = parts.uri.query().unwrap_or("").to_owned();

        println!("PARTS: {:?}", parts);
        let mut req = HttpRequest::new(
            parts.method, path, parts.version, parts.headers, query);
        let (psender, payload) = Payload::new(false);

        // start request processing
        let mut task = None;
        for h in router.iter() {
            if req.path().starts_with(h.prefix()) {
                task = Some(h.handle(&mut req, payload));
                break
            }
        }
        println!("REQ: {:?}", req);

        Entry {task: task.unwrap_or_else(|| Task::reply(HTTPNotFound)),
               req: UnsafeCell::new(req),
               payload: psender,
               recv: recv,
               respond: resp,
               eof: false,
               error: false,
               finished: false}
    }
}

struct IoWrapper<T> {
    unread: Option<Bytes>,
    inner: T,
}

impl<T: Read> Read for IoWrapper<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(mut bytes) = self.unread.take() {
            let size = cmp::min(buf.len(), bytes.len());
            buf[..size].copy_from_slice(&bytes[..size]);
            if bytes.len() > size {
                bytes.split_to(size);
                self.unread = Some(bytes);
            }
            Ok(size)
        } else {
            self.inner.read(buf)
        }
    }
}

impl<T: Write> Write for IoWrapper<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: AsyncRead + 'static> AsyncRead for IoWrapper<T> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.inner.prepare_uninitialized_buffer(buf)
    }
}

impl<T: AsyncWrite + 'static> AsyncWrite for IoWrapper<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.inner.shutdown()
    }
    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.inner.write_buf(buf)
    }
}

#![allow(
    dead_code, unused_imports, unused_imports, unreachable_code, unreachable_code,
    unused_variables
)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use actix::Arbiter;
use bytes::{Bytes, BytesMut};
use futures::task::current;
use futures::{Async, Future, Poll};
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, HttpTryFrom, Method, Uri, Version};
use httparse;
use tokio_core::reactor::Timeout;

use error::{ParseError, PayloadError, ResponseError};
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use payload::{Payload, PayloadStatus, PayloadWriter};
use pipeline::Pipeline;
use uri::Url;

use super::encoding::PayloadType;
use super::h1decoder::{DecoderError, Message};
use super::h1writer::H1Writer;
use super::settings::WorkerSettings;
use super::worker::IoWriter;
use super::{utils, Writer};
use super::{HttpHandler, HttpHandlerTask};
use io::{IoCommand, IoStream};

const MAX_BUFFER_SIZE: usize = 131_072;
const MAX_HEADERS: usize = 96;
const MAX_PIPELINED_MESSAGES: usize = 16;

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const ERROR = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const SHUTDOWN = 0b0000_1000;
        const DISCONNECTED = 0b0001_0000;
    }
}

bitflags! {
    struct EntryFlags: u8 {
        const EOF = 0b0000_0001;
        const ERROR = 0b0000_0010;
        const FINISHED = 0b0000_0100;
    }
}

pub(crate) struct Http1<H: 'static> {
    flags: Flags,
    settings: Rc<WorkerSettings<H>>,
    addr: Option<SocketAddr>,
    stream: IoStream,
    writer: H1Writer<H>,
    payload: Option<PayloadType>,
    tasks: VecDeque<Entry>,
    others: VecDeque<Entry>,
    keepalive_timer: Option<Timeout>,
}

struct Entry {
    pipe: Box<HttpHandlerTask>,
    flags: EntryFlags,
}

impl<H> Http1<H>
where
    H: HttpHandler + 'static,
{
    pub fn new(
        settings: Rc<WorkerSettings<H>>, stream: IoStream, writer: IoWriter,
    ) -> Self {
        let addr = stream.peer();
        let token = stream.token();
        let bytes = settings.get_shared_bytes();
        let writer = H1Writer::new(token, writer, bytes, Rc::clone(&settings));
        Http1 {
            addr,
            stream,
            settings,
            writer,
            flags: Flags::KEEPALIVE,
            tasks: VecDeque::new(),
            others: VecDeque::new(),
            payload: None,
            keepalive_timer: None,
        }
    }

    pub fn settings(&self) -> &WorkerSettings<H> {
        self.settings.as_ref()
    }

    #[inline]
    fn need_read(&self) -> PayloadStatus {
        if let Some(ref info) = self.payload {
            info.need_read()
        } else {
            PayloadStatus::Read
        }
    }

    fn poll_stream(&mut self) {
        //println!("STREAM");

        'outter: loop {
            match self.stream.try_recv() {
                Some(Ok(Message::Message {
                    msg,
                    payload,
                })) => {
                    if payload {
                        let (ps, pl) = Payload::new(false);
                        msg.get_mut().payload = Some(pl);
                        self.payload =
                            Some(PayloadType::new(&msg.get_ref().headers, ps));
                    }

                    let mut req = HttpRequest::from_message(msg);
                    //println!("{:?}", req);

                    // search handler for request
                    for h in self.settings.handlers().iter_mut() {
                        req = match h.handle(req) {
                            Ok(pipe) => {
                                self.tasks.push_back(Entry {
                                    pipe,
                                    flags: EntryFlags::empty(),
                                });
                                continue 'outter;
                            }
                            Err(req) => req,
                        }
                    }

                    // handler is not found
                    self.tasks.push_back(Entry {
                        pipe: Pipeline::error(HttpResponse::NotFound()),
                        flags: EntryFlags::empty(),
                    });
                }
                Some(Ok(Message::Chunk(chunk))) => {
                    if let Some(ref mut payload) = self.payload {
                        payload.feed_data(chunk);
                    } else {
                        panic!("");
                    }
                }
                Some(Ok(Message::Eof)) => {
                    if let Some(ref mut payload) = self.payload {
                        payload.feed_eof();
                    } else {
                        panic!("");
                    }
                }
                Some(Ok(Message::Hup)) => {
                    self.writer.done(false);
                    self.flags.insert(Flags::DISCONNECTED);
                    if let Some(ref mut payload) = self.payload {
                        payload.set_error(PayloadError::Incomplete);
                    }
                    break;
                }
                Some(Err(e)) => {
                    self.writer.done(false);
                    self.flags.insert(Flags::ERROR);
                    if let Some(ref mut payload) = self.payload {
                        let e = match e {
                            DecoderError::Io(e) => PayloadError::Io(e),
                            DecoderError::Error(e) => PayloadError::EncodingCorrupted,
                        };
                        payload.set_error(e);
                    }
                }
                None => break,
            }
        }
    }

    fn poll_io(&mut self) -> Poll<bool, ()> {
        //println!("IO");

        let retry = self.need_read() == PayloadStatus::Read;

        // check in-flight messages
        let mut io = false;
        let mut idx = 0;
        while idx < self.tasks.len() {
            let item = &mut self.tasks[idx];

            if !io && !item.flags.contains(EntryFlags::EOF) {
                // io is corrupted, send buffer
                if item.flags.contains(EntryFlags::ERROR) {
                    self.writer.done(true);
                    return Err(());
                }

                match item.pipe.poll_io(&mut self.writer) {
                    Ok(Async::Ready(ready)) => {
                        // override keep-alive state
                        // if self.stream.keepalive() {
                        // self.flags.insert(Flags::KEEPALIVE);
                        // } else {
                        // self.flags.remove(Flags::KEEPALIVE);
                        // }
                        // prepare stream for next response
                        // self.stream.reset();

                        if ready {
                            item.flags.insert(EntryFlags::EOF | EntryFlags::FINISHED);
                        } else {
                            item.flags.insert(EntryFlags::FINISHED);
                        }
                    }
                    // no more IO for this iteration
                    Ok(Async::NotReady) => {
                        //if self.need_read() == PayloadStatus::Read && !retry {
                        //self.writer.resume();
                        //}
                        io = true;
                    }
                    Err(err) => {
                        // it is not possible to recover from error
                        // during pipe handling, so just drop connection
                        error!("Unhandled error: {}", err);
                        item.flags.insert(EntryFlags::ERROR);

                        // check stream state, we still can have valid data in buffer
                        self.writer.done(true);

                        return Err(());
                    }
                }
            } else if !item.flags.contains(EntryFlags::FINISHED) {
                match item.pipe.poll() {
                    Ok(Async::NotReady) => (),
                    Ok(Async::Ready(_)) => item.flags.insert(EntryFlags::FINISHED),
                    Err(err) => {
                        item.flags.insert(EntryFlags::ERROR);
                        error!("Unhandled handler error: {}", err);
                    }
                }
            }
            idx += 1;
        }

        // cleanup finished tasks
        let mut popped = false;
        while !self.tasks.is_empty() {
            if self.tasks[0].flags.contains(EntryFlags::EOF | EntryFlags::FINISHED) {
                popped = true;
                self.tasks.pop_front();
            } else {
                break;
            }
        }

        // check stream state
        if self.flags.contains(Flags::STARTED) {
            match self.writer.poll_completed(false) {
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    return Err(());
                }
                _ => (),
            }
        }

        Ok(Async::NotReady)
    }
}

impl<H> Future for Http1<H>
where
    H: HttpHandler + 'static,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        //println!("HTTP1 POLL");

        // started
        if !self.flags.contains(Flags::STARTED) {
            self.flags.insert(Flags::STARTED);
            self.stream.set_notify(current());
        }

        loop {
            // process input stream
            if !self.flags.contains(Flags::DISCONNECTED) {
                self.poll_stream();
            }

            match self.poll_io()? {
                Async::Ready(true) => (),
                Async::Ready(false) => return Ok(Async::Ready(())),
                Async::NotReady => return Ok(Async::NotReady),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::{Buf, Bytes, BytesMut};
    use futures::{Async, Stream};
    use http::{Method, Version};
    use std::net::Shutdown;
    use std::{cmp, io, time};
    use tokio_io::{AsyncRead, AsyncWrite};

    use super::*;
    use application::HttpApplication;
    use httpmessage::HttpMessage;
    use server::settings::WorkerSettings;
    use server::{IoStream, KeepAlive};

    struct Buffer {
        buf: Bytes,
        err: Option<io::Error>,
    }

    impl Buffer {
        fn new(data: &'static str) -> Buffer {
            Buffer {
                buf: Bytes::from(data),
                err: None,
            }
        }
        fn feed_data(&mut self, data: &'static str) {
            let mut b = BytesMut::from(self.buf.as_ref());
            b.extend(data.as_bytes());
            self.buf = b.take().freeze();
        }
    }

    impl AsyncRead for Buffer {}
    impl io::Read for Buffer {
        fn read(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
            if self.buf.is_empty() {
                if self.err.is_some() {
                    Err(self.err.take().unwrap())
                } else {
                    Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
                }
            } else {
                let size = cmp::min(self.buf.len(), dst.len());
                let b = self.buf.split_to(size);
                dst[..size].copy_from_slice(&b);
                Ok(size)
            }
        }
    }

    impl IoStream for Buffer {
        fn shutdown(&mut self, _: Shutdown) -> io::Result<()> {
            Ok(())
        }
        fn set_nodelay(&mut self, _: bool) -> io::Result<()> {
            Ok(())
        }
        fn set_linger(&mut self, _: Option<time::Duration>) -> io::Result<()> {
            Ok(())
        }
    }
    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl AsyncWrite for Buffer {
        fn shutdown(&mut self) -> Poll<(), io::Error> {
            Ok(Async::Ready(()))
        }
        fn write_buf<B: Buf>(&mut self, _: &mut B) -> Poll<usize, io::Error> {
            Ok(Async::NotReady)
        }
    }

    macro_rules! not_ready {
        ($e:expr) => {
            match $e {
                Ok(Async::NotReady) => (),
                Err(err) => unreachable!("Unexpected error: {:?}", err),
                _ => unreachable!("Should not be ready"),
            }
        };
    }

    macro_rules! parse_ready {
        ($e:expr) => {{
            let settings: WorkerSettings<HttpApplication> =
                WorkerSettings::new(Vec::new(), KeepAlive::Os);
            match Reader::new().parse($e, &mut BytesMut::new(), &settings) {
                Ok(Async::Ready(req)) => req,
                Ok(_) => unreachable!("Eof during parsing http request"),
                Err(err) => unreachable!("Error during parsing http request: {:?}", err),
            }
        }};
    }

    macro_rules! reader_parse_ready {
        ($e:expr) => {
            match $e {
                Ok(Async::Ready(req)) => req,
                Ok(_) => unreachable!("Eof during parsing http request"),
                Err(err) => {
                    unreachable!("Error during parsing http request: {:?}", err)
                }
            }
        };
    }

    macro_rules! expect_parse_err {
        ($e:expr) => {{
            let mut buf = BytesMut::new();
            let settings: WorkerSettings<HttpApplication> =
                WorkerSettings::new(Vec::new(), KeepAlive::Os);

            match Reader::new().parse($e, &mut buf, &settings) {
                Err(err) => match err {
                    ReaderError::Error(_) => (),
                    _ => unreachable!("Parse error expected"),
                },
                _ => unreachable!("Error expected"),
            }
        }};
    }

    #[test]
    fn test_parse() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(req)) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial() {
        let mut buf = Buffer::new("PUT /test HTTP/1");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::NotReady) => (),
            _ => unreachable!("Error"),
        }

        buf.feed_data(".1\r\n\r\n");
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(req)) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::PUT);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_post() {
        let mut buf = Buffer::new("POST /test2 HTTP/1.0\r\n\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(req)) => {
                assert_eq!(req.version(), Version::HTTP_10);
                assert_eq!(*req.method(), Method::POST);
                assert_eq!(req.path(), "/test2");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(mut req)) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"body");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body_crlf() {
        let mut buf =
            Buffer::new("\r\nGET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(mut req)) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"body");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial_eof() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("\r\n");
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(req)) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_split_field() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n");
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("t");
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("es");
        not_ready!{ reader.parse(&mut buf, &mut readbuf, &settings) }

        buf.feed_data("t: value\r\n\r\n");
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(req)) => {
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(req.headers().get("test").unwrap().as_bytes(), b"value");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_multi_value() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             Set-Cookie: c1=cookie1\r\n\
             Set-Cookie: c2=cookie2\r\n\r\n",
        );
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        match reader.parse(&mut buf, &mut readbuf, &settings) {
            Ok(Async::Ready(req)) => {
                let val: Vec<_> = req.headers()
                    .get_all("Set-Cookie")
                    .iter()
                    .map(|v| v.to_str().unwrap().to_owned())
                    .collect();
                assert_eq!(val[0], "c1=cookie1");
                assert_eq!(val[1], "c2=cookie2");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_conn_default_1_0() {
        let mut buf = Buffer::new("GET /test HTTP/1.0\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_default_1_1() {
        let mut buf = Buffer::new("GET /test HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_close() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: close\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_close_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: close\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: keep-alive\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: keep-alive\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_0() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.0\r\n\
             connection: other\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: other\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_upgrade() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             upgrade: websockets\r\n\
             connection: upgrade\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.payload().eof());
        assert!(req.upgrade());
    }

    #[test]
    fn test_conn_upgrade_connect_method() {
        let mut buf = Buffer::new(
            "CONNECT /test HTTP/1.1\r\n\
             content-type: text/plain\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.upgrade());
        assert!(!req.payload().eof());
    }

    #[test]
    fn test_request_chunked() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        if let Ok(val) = req.chunked() {
            assert!(val);
        } else {
            unreachable!("Error");
        }

        // type in chunked
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chnked\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        if let Ok(val) = req.chunked() {
            assert!(!val);
        } else {
            unreachable!("Error");
        }
    }

    #[test]
    fn test_headers_content_length_err_1() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             content-length: line\r\n\r\n",
        );

        expect_parse_err!(&mut buf)
    }

    #[test]
    fn test_headers_content_length_err_2() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             content-length: -1\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_header() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test line\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_name() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test[]: line\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_bad_status_line() {
        let mut buf = Buffer::new("getpath \r\n\r\n");
        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_upgrade() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\
             upgrade: websocket\r\n\r\n\
             some raw data",
        );
        let mut req = parse_ready!(&mut buf);
        assert!(!req.keep_alive());
        assert!(req.upgrade());
        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"some raw data");
    }

    #[test]
    fn test_http_request_parser_utf8() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             x-test: тест\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert_eq!(
            req.headers().get("x-test").unwrap().as_bytes(),
            "тест".as_bytes()
        );
    }

    #[test]
    fn test_http_request_parser_two_slashes() {
        let mut buf = Buffer::new("GET //path HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert_eq!(req.path(), "//path");
    }

    #[test]
    fn test_http_request_parser_bad_method() {
        let mut buf = Buffer::new("!12%()+=~$ /get HTTP/1.1\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_parser_bad_version() {
        let mut buf = Buffer::new("GET //get HT/11\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_chunked_payload() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        let mut req =
            reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data("4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
        let _ = req.payload_mut().poll();
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(!req.payload().eof());
        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(req.payload().eof());
    }

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();

        let mut req =
            reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data(
            "4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
             POST /test2 HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let _ = req.payload_mut().poll();

        let req2 = reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert_eq!(*req2.method(), Method::POST);
        assert!(req2.chunked().unwrap());
        assert!(!req2.payload().eof());

        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(req.payload().eof());
    }

    #[test]
    fn test_http_request_chunked_payload_chunks() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        let mut req =
            reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        req.payload_mut().set_read_buffer_capacity(0);
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data("4\r\n1111\r\n");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"1111");

        buf.feed_data("4\r\ndata\r");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("\n4");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("\r");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        buf.feed_data("\n");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("li");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        buf.feed_data("ne\r\n0\r\n");
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        //trailers
        //buf.feed_data("test: test\r\n");
        //not_ready!(reader.parse(&mut buf, &mut readbuf));

        let _ = req.payload_mut().poll();
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));

        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(!req.payload().eof());

        buf.feed_data("\r\n");
        let _ = req.payload_mut().poll();
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.payload().eof());
    }

    #[test]
    fn test_parse_chunked_payload_chunk_extension() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let mut readbuf = BytesMut::new();
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = Reader::new();
        let mut req =
            reader_parse_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(req.chunked().unwrap());
        assert!(!req.payload().eof());

        buf.feed_data("4;test\r\ndata\r\n4\r\nline\r\n0\r\n\r\n"); // test: test\r\n\r\n")
        let _ = req.payload_mut().poll();
        not_ready!(reader.parse(&mut buf, &mut readbuf, &settings));
        assert!(!req.payload().eof());
        assert_eq!(req.payload_mut().readall().unwrap().as_ref(), b"dataline");
        assert!(req.payload().eof());
    }

    /*#[test]
    #[should_panic]
    fn test_parse_multiline() {
        let mut buf = Buffer::new(
            "GET /test HTTP/1.1\r\n\
             test: line\r\n \
               continue\r\n\
             test2: data\r\n\
             \r\n", false);

        let mut reader = Reader::new();
        match reader.parse(&mut buf) {
            Ok(res) => (),
            Err(err) => unreachable!("{:?}", err),
        }
    }*/
}

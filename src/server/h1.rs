use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

use actix::Arbiter;
use bytes::{BufMut, BytesMut};
use futures::{Async, Future, Poll};
use tokio_core::reactor::Timeout;

use error::PayloadError;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use payload::{Payload, PayloadStatus, PayloadWriter};
use pipeline::Pipeline;

use super::encoding::PayloadType;
use super::h1decoder::{DecoderError, H1Decoder, Message};
use super::h1writer::H1Writer;
use super::settings::WorkerSettings;
use super::Writer;
use super::{HttpHandler, HttpHandlerTask, IoStream};

const MAX_PIPELINED_MESSAGES: usize = 16;
const LW_BUFFER_SIZE: usize = 4096;
const HW_BUFFER_SIZE: usize = 32_768;

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

pub(crate) struct Http1<T: IoStream, H: 'static> {
    flags: Flags,
    settings: Rc<WorkerSettings<H>>,
    addr: Option<SocketAddr>,
    stream: H1Writer<T, H>,
    decoder: H1Decoder,
    payload: Option<PayloadType>,
    buf: BytesMut,
    tasks: VecDeque<Entry>,
    keepalive_timer: Option<Timeout>,
}

struct Entry {
    pipe: Box<HttpHandlerTask>,
    flags: EntryFlags,
}

impl<T, H> Http1<T, H>
where
    T: IoStream,
    H: HttpHandler + 'static,
{
    pub fn new(
        settings: Rc<WorkerSettings<H>>, stream: T, addr: Option<SocketAddr>,
        buf: BytesMut,
    ) -> Self {
        let bytes = settings.get_shared_bytes();
        Http1 {
            flags: Flags::KEEPALIVE,
            stream: H1Writer::new(stream, bytes, Rc::clone(&settings)),
            decoder: H1Decoder::new(),
            payload: None,
            tasks: VecDeque::new(),
            keepalive_timer: None,
            addr,
            buf,
            settings,
        }
    }

    #[inline]
    pub fn settings(&self) -> &WorkerSettings<H> {
        self.settings.as_ref()
    }

    #[inline]
    pub(crate) fn io(&mut self) -> &mut T {
        self.stream.get_mut()
    }

    #[inline]
    fn can_read(&self) -> bool {
        if let Some(ref info) = self.payload {
            info.need_read() == PayloadStatus::Read
        } else {
            true
        }
    }

    #[inline]
    pub fn poll(&mut self) -> Poll<(), ()> {
        // keep-alive timer
        if let Some(ref mut timer) = self.keepalive_timer {
            match timer.poll() {
                Ok(Async::Ready(_)) => {
                    trace!("Keep-alive timeout, close connection");
                    self.flags.insert(Flags::SHUTDOWN);
                }
                Ok(Async::NotReady) => (),
                Err(_) => unreachable!(),
            }
        }

        // shutdown
        if self.flags.contains(Flags::SHUTDOWN) {
            match self.stream.poll_completed(true) {
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Ok(Async::Ready(_)) => return Ok(Async::Ready(())),
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    return Err(());
                }
            }
        }

        self.poll_io();

        loop {
            match self.poll_handler()? {
                Async::Ready(true) => {
                    self.poll_io();
                }
                Async::Ready(false) => {
                    self.flags.insert(Flags::SHUTDOWN);
                    return self.poll();
                }
                Async::NotReady => return Ok(Async::NotReady),
            }
        }
    }

    #[inline]
    pub fn poll_io(&mut self) {
        // read io from socket
        if !self.flags.intersects(Flags::ERROR)
            && self.tasks.len() < MAX_PIPELINED_MESSAGES && self.can_read()
        {
            if self.read() {
                // notify all tasks
                self.stream.disconnected();
                for entry in &mut self.tasks {
                    entry.pipe.disconnected()
                }
                // kill keepalive
                self.flags.remove(Flags::KEEPALIVE);
                self.keepalive_timer.take();

                // on parse error, stop reading stream but tasks need to be
                // completed
                self.flags.insert(Flags::ERROR);

                if let Some(mut payload) = self.payload.take() {
                    payload.set_error(PayloadError::Incomplete);
                }
            } else {
                self.parse();
            }
        }
    }

    pub fn poll_handler(&mut self) -> Poll<bool, ()> {
        let retry = self.can_read();

        // check in-flight messages
        let mut io = false;
        let mut idx = 0;
        while idx < self.tasks.len() {
            let item: &mut Entry = unsafe { &mut *(&mut self.tasks[idx] as *mut _) };

            // only one task can do io operation in http/1
            if !io && !item.flags.contains(EntryFlags::EOF) {
                // io is corrupted, send buffer
                if item.flags.contains(EntryFlags::ERROR) {
                    if let Ok(Async::NotReady) = self.stream.poll_completed(true) {
                        return Ok(Async::NotReady);
                    }
                    return Err(());
                }

                match item.pipe.poll_io(&mut self.stream) {
                    Ok(Async::Ready(ready)) => {
                        // override keep-alive state
                        if self.stream.keepalive() {
                            self.flags.insert(Flags::KEEPALIVE);
                        } else {
                            self.flags.remove(Flags::KEEPALIVE);
                        }
                        // prepare stream for next response
                        self.stream.reset();

                        if ready {
                            item.flags
                                .insert(EntryFlags::EOF | EntryFlags::FINISHED);
                        } else {
                            item.flags.insert(EntryFlags::FINISHED);
                        }
                    }
                    // no more IO for this iteration
                    Ok(Async::NotReady) => {
                        // check if previously read backpressure was enabled
                        if self.can_read() && !retry {
                            return Ok(Async::Ready(true));
                        }
                        io = true;
                    }
                    Err(err) => {
                        // it is not possible to recover from error
                        // during pipe handling, so just drop connection
                        error!("Unhandled error: {}", err);
                        item.flags.insert(EntryFlags::ERROR);

                        // check stream state, we still can have valid data in buffer
                        if let Ok(Async::NotReady) = self.stream.poll_completed(true) {
                            return Ok(Async::NotReady);
                        }
                        return Err(());
                    }
                }
            } else if !item.flags.contains(EntryFlags::FINISHED) {
                match item.pipe.poll() {
                    Ok(Async::NotReady) => (),
                    Ok(Async::Ready(_)) => item.flags.insert(EntryFlags::FINISHED),
                    Err(err) => {
                        item.flags.insert(EntryFlags::ERROR);
                        error!("Unhandled error: {}", err);
                    }
                }
            }
            idx += 1;
        }

        // cleanup finished tasks
        let max = self.tasks.len() >= MAX_PIPELINED_MESSAGES;
        while !self.tasks.is_empty() {
            if self.tasks[0]
                .flags
                .contains(EntryFlags::EOF | EntryFlags::FINISHED)
            {
                self.tasks.pop_front();
            } else {
                break;
            }
        }
        // read more message
        if max && self.tasks.len() >= MAX_PIPELINED_MESSAGES {
            return Ok(Async::Ready(true));
        }

        // check stream state
        if self.flags.contains(Flags::STARTED) {
            match self.stream.poll_completed(false) {
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(err) => {
                    debug!("Error sending data: {}", err);
                    return Err(());
                }
                _ => (),
            }
        }

        // deal with keep-alive
        if self.tasks.is_empty() {
            // no keep-alive
            if self.flags.contains(Flags::ERROR)
                || (!self.flags.contains(Flags::KEEPALIVE)
                    || !self.settings.keep_alive_enabled())
                    && self.flags.contains(Flags::STARTED)
            {
                return Ok(Async::Ready(false));
            }

            // start keep-alive timer
            let keep_alive = self.settings.keep_alive();
            if self.keepalive_timer.is_none() && keep_alive > 0 {
                trace!("Start keep-alive timer");
                let mut timer =
                    Timeout::new(Duration::new(keep_alive, 0), Arbiter::handle())
                        .unwrap();
                // register timer
                let _ = timer.poll();
                self.keepalive_timer = Some(timer);
            }
        }
        Ok(Async::NotReady)
    }

    pub fn parse(&mut self) {
        'outer: loop {
            match self.decoder.decode(&mut self.buf, &self.settings) {
                Ok(Some(Message::Message { msg, payload })) => {
                    self.flags.insert(Flags::STARTED);

                    if payload {
                        let (ps, pl) = Payload::new(false);
                        msg.get_mut().payload = Some(pl);
                        self.payload =
                            Some(PayloadType::new(&msg.get_ref().headers, ps));
                    }

                    let mut req = HttpRequest::from_message(msg);

                    // set remote addr
                    req.set_peer_addr(self.addr);

                    // stop keepalive timer
                    self.keepalive_timer.take();

                    // search handler for request
                    for h in self.settings.handlers().iter_mut() {
                        req = match h.handle(req) {
                            Ok(pipe) => {
                                self.tasks.push_back(Entry {
                                    pipe,
                                    flags: EntryFlags::empty(),
                                });
                                continue 'outer;
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
                Ok(Some(Message::Chunk(chunk))) => {
                    if let Some(ref mut payload) = self.payload {
                        payload.feed_data(chunk);
                    } else {
                        error!("Internal server error: unexpected payload chunk");
                        self.flags.insert(Flags::ERROR);
                    }
                }
                Ok(Some(Message::Eof)) => {
                    if let Some(mut payload) = self.payload.take() {
                        payload.feed_eof();
                    } else {
                        error!("Internal server error: unexpected eof");
                        self.flags.insert(Flags::ERROR);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    self.flags.insert(Flags::ERROR);
                    if let Some(mut payload) = self.payload.take() {
                        let e = match e {
                            DecoderError::Io(e) => PayloadError::Io(e),
                            DecoderError::Error(_) => PayloadError::EncodingCorrupted,
                        };
                        payload.set_error(e);
                    }
                }
            }
        }
    }

    #[inline]
    fn read(&mut self) -> bool {
        loop {
            unsafe {
                if self.buf.remaining_mut() < LW_BUFFER_SIZE {
                    self.buf.reserve(HW_BUFFER_SIZE);
                }
                match self.stream.get_mut().read(self.buf.bytes_mut()) {
                    Ok(n) => {
                        if n == 0 {
                            return true;
                        } else {
                            self.buf.advance_mut(n);
                        }
                    }
                    Err(e) => {
                        return e.kind() != io::ErrorKind::WouldBlock;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::{Bytes, BytesMut};
    use http::{Method, Version};

    use super::*;
    use application::HttpApplication;
    use httpmessage::HttpMessage;
    use server::h1decoder::Message;
    use server::helpers::SharedHttpInnerMessage;
    use server::settings::WorkerSettings;
    use server::KeepAlive;

    impl Message {
        fn message(self) -> SharedHttpInnerMessage {
            match self {
                Message::Message { msg, payload: _ } => msg,
                _ => panic!("error"),
            }
        }
        fn is_payload(&self) -> bool {
            match *self {
                Message::Message { msg: _, payload } => payload,
                _ => panic!("error"),
            }
        }
        fn chunk(self) -> Bytes {
            match self {
                Message::Chunk(chunk) => chunk,
                _ => panic!("error"),
            }
        }
        fn eof(&self) -> bool {
            match *self {
                Message::Eof => true,
                _ => false,
            }
        }
    }

    macro_rules! parse_ready {
        ($e:expr) => {{
            let settings: WorkerSettings<HttpApplication> =
                WorkerSettings::new(Vec::new(), KeepAlive::Os);
            match H1Decoder::new().decode($e, &settings) {
                Ok(Some(msg)) => HttpRequest::from_message(msg.message()),
                Ok(_) => unreachable!("Eof during parsing http request"),
                Err(err) => unreachable!("Error during parsing http request: {:?}", err),
            }
        }};
    }

    macro_rules! expect_parse_err {
        ($e:expr) => {{
            let settings: WorkerSettings<HttpApplication> =
                WorkerSettings::new(Vec::new(), KeepAlive::Os);

            match H1Decoder::new().decode($e, &settings) {
                Err(err) => match err {
                    DecoderError::Error(_) => (),
                    _ => unreachable!("Parse error expected"),
                },
                _ => unreachable!("Error expected"),
            }
        }};
    }

    #[test]
    fn test_parse() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n\r\n");
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let req = HttpRequest::from_message(msg.message());
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial() {
        let mut buf = BytesMut::from("PUT /test HTTP/1");
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(None) => (),
            _ => unreachable!("Error"),
        }

        buf.extend(b".1\r\n\r\n");
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = HttpRequest::from_message(msg.message());
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::PUT);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_post() {
        let mut buf = BytesMut::from("POST /test2 HTTP/1.0\r\n\r\n");
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = HttpRequest::from_message(msg.message());
                assert_eq!(req.version(), Version::HTTP_10);
                assert_eq!(*req.method(), Method::POST);
                assert_eq!(req.path(), "/test2");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body() {
        let mut buf =
            BytesMut::from("GET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = HttpRequest::from_message(msg.message());
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(
                    reader
                        .decode(&mut buf, &settings)
                        .unwrap()
                        .unwrap()
                        .chunk()
                        .as_ref(),
                    b"body"
                );
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_body_crlf() {
        let mut buf =
            BytesMut::from("\r\nGET /test HTTP/1.1\r\nContent-Length: 4\r\n\r\nbody");
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let mut req = HttpRequest::from_message(msg.message());
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(
                    reader
                        .decode(&mut buf, &settings)
                        .unwrap()
                        .unwrap()
                        .chunk()
                        .as_ref(),
                    b"body"
                );
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_parse_partial_eof() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n");
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);
        let mut reader = H1Decoder::new();
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"\r\n");
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let req = HttpRequest::from_message(msg.message());
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_split_field() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n");
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        assert!{ reader.decode(&mut buf, &settings).unwrap().is_none() }

        buf.extend(b"t");
        assert!{ reader.decode(&mut buf, &settings).unwrap().is_none() }

        buf.extend(b"es");
        assert!{ reader.decode(&mut buf, &settings).unwrap().is_none() }

        buf.extend(b"t: value\r\n\r\n");
        match reader.decode(&mut buf, &settings) {
            Ok(Some(msg)) => {
                let req = HttpRequest::from_message(msg.message());
                assert_eq!(req.version(), Version::HTTP_11);
                assert_eq!(*req.method(), Method::GET);
                assert_eq!(req.path(), "/test");
                assert_eq!(
                    req.headers().get("test").unwrap().as_bytes(),
                    b"value"
                );
            }
            Ok(_) | Err(_) => unreachable!("Error during parsing http request"),
        }
    }

    #[test]
    fn test_headers_multi_value() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             Set-Cookie: c1=cookie1\r\n\
             Set-Cookie: c2=cookie2\r\n\r\n",
        );
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);
        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        let req = HttpRequest::from_message(msg.message());

        let val: Vec<_> = req.headers()
            .get_all("Set-Cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(val[0], "c1=cookie1");
        assert_eq!(val[1], "c2=cookie2");
    }

    #[test]
    fn test_conn_default_1_0() {
        let mut buf = BytesMut::from("GET /test HTTP/1.0\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_default_1_1() {
        let mut buf = BytesMut::from("GET /test HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_close() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: close\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_close_1_0() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: close\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_0() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: keep-alive\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_keep_alive_1_1() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: keep-alive\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_0() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.0\r\n\
             connection: other\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(!req.keep_alive());
    }

    #[test]
    fn test_conn_other_1_1() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: other\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.keep_alive());
    }

    #[test]
    fn test_conn_upgrade() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             upgrade: websockets\r\n\
             connection: upgrade\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.upgrade());
    }

    #[test]
    fn test_conn_upgrade_connect_method() {
        let mut buf = BytesMut::from(
            "CONNECT /test HTTP/1.1\r\n\
             content-type: text/plain\r\n\r\n",
        );
        let req = parse_ready!(&mut buf);

        assert!(req.upgrade());
    }

    #[test]
    fn test_request_chunked() {
        let mut buf = BytesMut::from(
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
        let mut buf = BytesMut::from(
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
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             content-length: line\r\n\r\n",
        );

        expect_parse_err!(&mut buf)
    }

    #[test]
    fn test_headers_content_length_err_2() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             content-length: -1\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_header() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             test line\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_invalid_name() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             test[]: line\r\n\r\n",
        );

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_bad_status_line() {
        let mut buf = BytesMut::from("getpath \r\n\r\n");
        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_upgrade() {
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             connection: upgrade\r\n\
             upgrade: websocket\r\n\r\n\
             some raw data",
        );
        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = HttpRequest::from_message(msg.message());
        assert!(!req.keep_alive());
        assert!(req.upgrade());
        assert_eq!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .chunk()
                .as_ref(),
            b"some raw data"
        );
    }

    #[test]
    fn test_http_request_parser_utf8() {
        let mut buf = BytesMut::from(
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
        let mut buf = BytesMut::from("GET //path HTTP/1.1\r\n\r\n");
        let req = parse_ready!(&mut buf);

        assert_eq!(req.path(), "//path");
    }

    #[test]
    fn test_http_request_parser_bad_method() {
        let mut buf = BytesMut::from("!12%()+=~$ /get HTTP/1.1\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_parser_bad_version() {
        let mut buf = BytesMut::from("GET //get HT/11\r\n\r\n");

        expect_parse_err!(&mut buf);
    }

    #[test]
    fn test_http_request_chunked_payload() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = HttpRequest::from_message(msg.message());
        assert!(req.chunked().unwrap());

        buf.extend(b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n");
        assert_eq!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .chunk()
                .as_ref(),
            b"data"
        );
        assert_eq!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .chunk()
                .as_ref(),
            b"line"
        );
        assert!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .eof()
        );
    }

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);
        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = HttpRequest::from_message(msg.message());
        assert!(req.chunked().unwrap());

        buf.extend(
            b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
              POST /test2 HTTP/1.1\r\n\
              transfer-encoding: chunked\r\n\r\n"
                .iter(),
        );
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"line");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.eof());

        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req2 = HttpRequest::from_message(msg.message());
        assert!(req2.chunked().unwrap());
        assert_eq!(*req2.method(), Method::POST);
        assert!(req2.chunked().unwrap());
    }

    #[test]
    fn test_http_request_chunked_payload_chunks() {
        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = HttpRequest::from_message(msg.message());
        assert!(req.chunked().unwrap());

        buf.extend(b"4\r\n1111\r\n");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"1111");

        buf.extend(b"4\r\ndata\r");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");

        buf.extend(b"\n4");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"\r");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());
        buf.extend(b"\n");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"li");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"li");

        //trailers
        //buf.feed_data("test: test\r\n");
        //not_ready!(reader.parse(&mut buf, &mut readbuf));

        buf.extend(b"ne\r\n0\r\n");
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"ne");
        assert!(reader.decode(&mut buf, &settings).unwrap().is_none());

        buf.extend(b"\r\n");
        assert!(
            reader
                .decode(&mut buf, &settings)
                .unwrap()
                .unwrap()
                .eof()
        );
    }

    #[test]
    fn test_parse_chunked_payload_chunk_extension() {
        let mut buf = BytesMut::from(
            &"GET /test HTTP/1.1\r\n\
              transfer-encoding: chunked\r\n\r\n"[..],
        );
        let settings = WorkerSettings::<HttpApplication>::new(Vec::new(), KeepAlive::Os);

        let mut reader = H1Decoder::new();
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.is_payload());
        let req = HttpRequest::from_message(msg.message());
        assert!(req.chunked().unwrap());

        buf.extend(b"4;test\r\ndata\r\n4\r\nline\r\n0\r\n\r\n"); // test: test\r\n\r\n")
        let chunk = reader
            .decode(&mut buf, &settings)
            .unwrap()
            .unwrap()
            .chunk();
        assert_eq!(chunk, Bytes::from_static(b"data"));
        let chunk = reader
            .decode(&mut buf, &settings)
            .unwrap()
            .unwrap()
            .chunk();
        assert_eq!(chunk, Bytes::from_static(b"line"));
        let msg = reader.decode(&mut buf, &settings).unwrap().unwrap();
        assert!(msg.eof());
    }
}

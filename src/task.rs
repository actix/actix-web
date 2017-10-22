use std::{cmp, io};
use std::rc::Rc;
use std::fmt::Write;
use std::collections::VecDeque;

use http::{StatusCode, Version};
use http::header::{HeaderValue,
                   CONNECTION, CONTENT_TYPE, CONTENT_LENGTH, TRANSFER_ENCODING, DATE};
use bytes::BytesMut;
use futures::{Async, Future, Poll, Stream};
use tokio_io::{AsyncRead, AsyncWrite};

use date;
use route::Frame;
use application::Middleware;
use httprequest::HttpRequest;
use httpresponse::{Body, HttpResponse};

type FrameStream = Stream<Item=Frame, Error=io::Error>;
const AVERAGE_HEADER_SIZE: usize = 30; // totally scientific
const MAX_WRITE_BUFFER_SIZE: usize = 65_536; // max buffer size 64k

#[derive(PartialEq, Debug)]
enum TaskRunningState {
    Paused,
    Running,
    Done,
}

impl TaskRunningState {
    fn is_done(&self) -> bool {
        *self == TaskRunningState::Done
    }
}

#[derive(PartialEq, Debug)]
enum TaskIOState {
    ReadingMessage,
    ReadingPayload,
    Done,
}

impl TaskIOState {
    fn is_done(&self) -> bool {
        *self == TaskIOState::Done
    }
}

pub struct Task {
    state: TaskRunningState,
    iostate: TaskIOState,
    frames: VecDeque<Frame>,
    stream: Option<Box<FrameStream>>,
    encoder: Encoder,
    buffer: BytesMut,
    upgrade: bool,
    keepalive: bool,
    prepared: Option<HttpResponse>,
    middlewares: Option<Rc<Vec<Box<Middleware>>>>,
}

impl Task {

    pub fn reply<R: Into<HttpResponse>>(response: R) -> Self {
        let mut frames = VecDeque::new();
        frames.push_back(Frame::Message(response.into()));
        frames.push_back(Frame::Payload(None));

        Task {
            state: TaskRunningState::Running,
            iostate: TaskIOState::Done,
            frames: frames,
            stream: None,
            encoder: Encoder::length(0),
            buffer: BytesMut::new(),
            upgrade: false,
            keepalive: false,
            prepared: None,
            middlewares: None,
        }
    }

    pub(crate) fn with_stream<S>(stream: S) -> Self
        where S: Stream<Item=Frame, Error=io::Error> + 'static
    {
        Task {
            state: TaskRunningState::Running,
            iostate: TaskIOState::ReadingMessage,
            frames: VecDeque::new(),
            stream: Some(Box::new(stream)),
            encoder: Encoder::length(0),
            buffer: BytesMut::new(),
            upgrade: false,
            keepalive: false,
            prepared: None,
            middlewares: None,
        }
    }

    pub(crate) fn keepalive(&self) -> bool {
        self.keepalive && !self.upgrade
    }

    pub(crate) fn set_middlewares(&mut self, middlewares: Rc<Vec<Box<Middleware>>>) {
        self.middlewares = Some(middlewares);
    }

    fn prepare(&mut self, req: &mut HttpRequest, msg: HttpResponse)
    {
        trace!("Prepare message status={:?}", msg.status);

        // run middlewares
        let mut msg = if let Some(middlewares) = self.middlewares.take() {
            let mut msg = msg;
            for middleware in middlewares.iter() {
                msg = middleware.response(req, msg);
            }
            self.middlewares = Some(middlewares);
            msg
        } else {
            msg
        };

        // prepare task
        let mut extra = 0;
        let body = msg.replace_body(Body::Empty);
        let version = msg.version().unwrap_or_else(|| req.version());
        self.keepalive = msg.keep_alive().unwrap_or_else(|| req.keep_alive());

        match body {
            Body::Empty => {
                if msg.chunked() {
                    error!("Chunked transfer is enabled but body is set to Empty");
                }
                msg.headers.insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
                msg.headers.remove(TRANSFER_ENCODING);
                self.encoder = Encoder::length(0);
            },
            Body::Length(n) => {
                if msg.chunked() {
                    error!("Chunked transfer is enabled but body with specific length is specified");
                }
                msg.headers.insert(
                    CONTENT_LENGTH,
                    HeaderValue::from_str(format!("{}", n).as_str()).unwrap());
                msg.headers.remove(TRANSFER_ENCODING);
                self.encoder = Encoder::length(n);
            },
            Body::Binary(ref bytes) => {
                extra = bytes.len();
                msg.headers.insert(
                    CONTENT_LENGTH,
                    HeaderValue::from_str(format!("{}", bytes.len()).as_str()).unwrap());
                msg.headers.remove(TRANSFER_ENCODING);
                self.encoder = Encoder::length(0);
            }
            Body::Streaming => {
                if msg.chunked() {
                    if version < Version::HTTP_11 {
                        error!("Chunked transfer encoding is forbidden for {:?}", version);
                    }
                    msg.headers.remove(CONTENT_LENGTH);
                    msg.headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                    self.encoder = Encoder::chunked();
                } else {
                    self.encoder = Encoder::eof();
                }
            }
            Body::Upgrade => {
                msg.headers.insert(CONNECTION, HeaderValue::from_static("upgrade"));
                self.encoder = Encoder::eof();
            }
        }

        // Connection upgrade
        if msg.upgrade() {
            msg.headers.insert(CONNECTION, HeaderValue::from_static("upgrade"));
        }
        // keep-alive
        else if self.keepalive {
            if version < Version::HTTP_11 {
                msg.headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
            }
        } else if version >= Version::HTTP_11 {
            msg.headers.insert(CONNECTION, HeaderValue::from_static("close"));
        }

        // render message
        let init_cap = 100 + msg.headers.len() * AVERAGE_HEADER_SIZE + extra;
        self.buffer.reserve(init_cap);

        if version == Version::HTTP_11 && msg.status == StatusCode::OK {
            self.buffer.extend(b"HTTP/1.1 200 OK\r\n");
        } else {
            let _ = write!(self.buffer, "{:?} {}\r\n", version, msg.status);
        }
        for (key, value) in &msg.headers {
            let t: &[u8] = key.as_ref();
            self.buffer.extend(t);
            self.buffer.extend(b": ");
            self.buffer.extend(value.as_ref());
            self.buffer.extend(b"\r\n");
        }

        // using http::h1::date is quite a lot faster than generating
        // a unique Date header each time like req/s goes up about 10%
        if !msg.headers.contains_key(DATE) {
            self.buffer.reserve(date::DATE_VALUE_LENGTH + 8);
            self.buffer.extend(b"Date: ");
            date::extend(&mut self.buffer);
            self.buffer.extend(b"\r\n");
        }

        // default content-type
        if !msg.headers.contains_key(CONTENT_TYPE) {
            self.buffer.extend(b"ContentType: application/octet-stream\r\n".as_ref());
        }

        self.buffer.extend(b"\r\n");

        if let Body::Binary(ref bytes) = body {
            self.buffer.extend(bytes);
            self.prepared = Some(msg);
            return
        }
        msg.replace_body(body);
        self.prepared = Some(msg);
    }

    pub(crate) fn poll_io<T>(&mut self, io: &mut T, req: &mut HttpRequest) -> Poll<bool, ()>
        where T: AsyncRead + AsyncWrite
    {
        trace!("POLL-IO frames:{:?}", self.frames.len());
        // response is completed
        if self.frames.is_empty() && self.iostate.is_done() {
            return Ok(Async::Ready(self.state.is_done()));
        } else {
            // poll stream
            if self.state == TaskRunningState::Running {
                match self.poll() {
                    Ok(Async::Ready(_)) => {
                        self.state = TaskRunningState::Done;
                    }
                    Ok(Async::NotReady) => (),
                    Err(_) => return Err(())
                }
            }

            // use exiting frames
            while let Some(frame) = self.frames.pop_front() {
                trace!("IO Frame: {:?}", frame);
                match frame {
                    Frame::Message(response) => {
                        self.prepare(req, response);
                    }
                    Frame::Payload(Some(chunk)) => {
                        if self.prepared.is_some() {
                            // TODO: add warning, write after EOF
                            self.encoder.encode(&mut self.buffer, chunk.as_ref());
                        } else {
                            // might be response for EXCEPT
                            self.buffer.extend(chunk)
                        }
                    },
                    Frame::Payload(None) => {
                        // TODO: add error "not eof""
                        if !self.encoder.encode(&mut self.buffer, [].as_ref()) {
                            debug!("last payload item, but it is not EOF ");
                            return Err(())
                        }
                        break
                    },
                }
            }
        }

        // write bytes to TcpStream
        while !self.buffer.is_empty() {
            match io.write(self.buffer.as_ref()) {
                Ok(n) => {
                    self.buffer.split_to(n);
                },
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    break
                }
                Err(_) => return Err(()),
            }
        }

        // should pause task
        if self.state != TaskRunningState::Done {
            if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
                self.state = TaskRunningState::Paused;
            } else if self.state == TaskRunningState::Paused {
                self.state = TaskRunningState::Running;
            }
        }

        // response is completed
        if self.buffer.is_empty() && self.iostate.is_done() {
            // run middlewares
            if let Some(ref mut resp) = self.prepared {
                if let Some(middlewares) = self.middlewares.take() {
                    for middleware in middlewares.iter() {
                        middleware.finish(req, resp);
                    }
                }
            }

            Ok(Async::Ready(self.state.is_done()))
        } else {
            Ok(Async::NotReady)
        }
    }
}

impl Future for Task {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(ref mut stream) = self.stream {
            loop {
                match stream.poll() {
                    Ok(Async::Ready(Some(frame))) => {
                        match frame {
                            Frame::Message(ref msg) => {
                                if self.iostate != TaskIOState::ReadingMessage {
                                    error!("Non expected frame {:?}", frame);
                                    return Err(())
                                }
                                self.upgrade = msg.upgrade();
                                if self.upgrade || msg.body().has_body() {
                                    self.iostate = TaskIOState::ReadingPayload;
                                } else {
                                    self.iostate = TaskIOState::Done;
                                }
                            },
                            Frame::Payload(ref chunk) => {
                                if chunk.is_none() {
                                    self.iostate = TaskIOState::Done;
                                } else if self.iostate != TaskIOState::ReadingPayload {
                                    error!("Non expected frame {:?}", self.iostate);
                                    return Err(())
                                }
                            },
                        }
                        self.frames.push_back(frame)
                    },
                    Ok(Async::Ready(None)) =>
                        return Ok(Async::Ready(())),
                    Ok(Async::NotReady) =>
                        return Ok(Async::NotReady),
                    Err(_) =>
                        return Err(())
                }
            }
        } else {
            Ok(Async::Ready(()))
        }
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug, Clone)]
struct Encoder {
    kind: Kind,
}

#[derive(Debug, PartialEq, Clone)]
enum Kind {
    /// An Encoder for when Transfer-Encoding includes `chunked`.
    Chunked(bool),
    /// An Encoder for when Content-Length is set.
    ///
    /// Enforces that the body is not longer than the Content-Length header.
    Length(u64),
    /// An Encoder for when Content-Length is not known.
    ///
    /// Appliction decides when to stop writing.
    Eof,
}

impl Encoder {

    pub fn eof() -> Encoder {
        Encoder {
            kind: Kind::Eof,
        }
    }

    pub fn chunked() -> Encoder {
        Encoder {
            kind: Kind::Chunked(false),
        }
    }

    pub fn length(len: u64) -> Encoder {
        Encoder {
            kind: Kind::Length(len),
        }
    }

    /*pub fn is_eof(&self) -> bool {
        match self.kind {
            Kind::Eof | Kind::Length(0) => true,
            Kind::Chunked(eof) => eof,
            _ => false,
        }
    }*/

    /// Encode message. Return `EOF` state of encoder
    pub fn encode(&mut self, dst: &mut BytesMut, msg: &[u8]) -> bool {
        match self.kind {
            Kind::Eof => {
                dst.extend(msg);
                msg.is_empty()
            },
            Kind::Chunked(ref mut eof) => {
                if *eof {
                    return true;
                }

                if msg.is_empty() {
                    *eof = true;
                    dst.extend(b"0\r\n\r\n");
                } else {
                    write!(dst, "{:X}\r\n", msg.len()).unwrap();
                    dst.extend(msg);
                    dst.extend(b"\r\n");
                }
                *eof
            },
            Kind::Length(ref mut remaining) => {
                if msg.is_empty() {
                    return *remaining == 0
                }
                let max = cmp::min(*remaining, msg.len() as u64);
                trace!("sized write = {}", max);
                dst.extend(msg[..max as usize].as_ref());

                *remaining -= max as u64;
                trace!("encoded {} bytes, remaining = {}", max, remaining);
                *remaining == 0
            },
        }
    }
}

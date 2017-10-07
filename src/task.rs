use std::{cmp, io};
use std::io::Write as IoWrite;
use std::fmt::Write;
use std::collections::VecDeque;

use http::{StatusCode, Version};
use bytes::{Bytes, BytesMut};
use futures::{Async, Future, Poll, Stream};
use tokio_core::net::TcpStream;

use hyper::header::{Date, Connection, ContentType,
                    ContentLength, Encoding, TransferEncoding};

use date;
use route::Frame;
use httpmessage::{Body, HttpMessage};

type FrameStream = Stream<Item=Frame, Error=io::Error>;
const AVERAGE_HEADER_SIZE: usize = 30; // totally scientific
const DEFAULT_LIMIT: usize = 65_536; // max buffer size 64k


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
}

impl Task {

    pub(crate) fn reply(msg: HttpMessage, body: Option<Bytes>) -> Self {
        let mut frames = VecDeque::new();
        if let Some(body) = body {
            frames.push_back(Frame::Message(msg));
            frames.push_back(Frame::Payload(Some(body)));
            frames.push_back(Frame::Payload(None));
        } else {
            frames.push_back(Frame::Message(msg));
        }

        Task {
            state: TaskRunningState::Running,
            iostate: TaskIOState::Done,
            frames: frames,
            stream: None,
            encoder: Encoder::length(0),
            buffer: BytesMut::new(),
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
        }
    }

    fn prepare(&mut self, mut msg: HttpMessage)
    {
        trace!("Prepare message status={:?}", msg.status);

        let mut extra = 0;
        let body = msg.set_body(Body::Empty);
        match body {
            Body::Empty => {
                if msg.chunked() {
                    error!("Chunked transfer is enabled but body is set to Empty");
                }
                msg.headers.set(ContentLength(0));
                msg.headers.remove::<TransferEncoding>();
                self.encoder = Encoder::length(0);
            },
            Body::Length(n) => {
                if msg.chunked() {
                    error!("Chunked transfer is enabled but body with specific length is specified");
                }
                msg.headers.set(ContentLength(n));
                msg.headers.remove::<TransferEncoding>();
                self.encoder = Encoder::length(n);
            },
            Body::Binary(ref bytes) => {
                extra = bytes.len();
                msg.headers.set(ContentLength(bytes.len() as u64));
                msg.headers.remove::<TransferEncoding>();
                self.encoder = Encoder::length(0);
            }
            Body::Streaming => {
                if msg.chunked() {
                    if msg.version < Version::HTTP_11 {
                        error!("Chunked transfer encoding is forbidden for {:?}", msg.version);
                    }
                    msg.headers.remove::<ContentLength>();
                    msg.headers.set(TransferEncoding(vec![Encoding::Chunked]));
                    self.encoder = Encoder::chunked();
                } else {
                    self.encoder = Encoder::eof();
                }
            }
        }

        // keep-alive
        if !msg.headers.has::<Connection>() {
            if msg.keep_alive() {
                if msg.version < Version::HTTP_11 {
                    msg.headers.set(Connection::keep_alive());
                }
            } else if msg.version >= Version::HTTP_11 {
                msg.headers.set(Connection::close());
            }
        }

        // render message
        let init_cap = 30 + msg.headers.len() * AVERAGE_HEADER_SIZE + extra;
        self.buffer.reserve(init_cap);

        if msg.version == Version::HTTP_11 && msg.status == StatusCode::OK {
            self.buffer.extend(b"HTTP/1.1 200 OK\r\n");
            let _ = write!(self.buffer, "{}", msg.headers);
        } else {
            let _ = write!(self.buffer, "{:?} {}\r\n{}", msg.version, msg.status, msg.headers);
        }
        // using http::h1::date is quite a lot faster than generating
        // a unique Date header each time like req/s goes up about 10%
        if !msg.headers.has::<Date>() {
            self.buffer.reserve(date::DATE_VALUE_LENGTH + 8);
            self.buffer.extend(b"Date: ");
            date::extend(&mut self.buffer);
            self.buffer.extend(b"\r\n");
        }

        // default content-type
        if !msg.headers.has::<ContentType>() {
            self.buffer.extend(b"ContentType: application/octet-stream\r\n".as_ref());
        }

        self.buffer.extend(b"\r\n");

        if let Body::Binary(ref bytes) = *msg.body() {
            self.buffer.extend(bytes);
            return
        }
        msg.set_body(body);
    }

    pub(crate) fn poll_io(&mut self, io: &mut TcpStream) -> Poll<bool, ()> {
        println!("POLL-IO {:?}", self.frames.len());
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
                match frame {
                    Frame::Message(message) => {
                        self.prepare(message);
                    }
                    Frame::Payload(chunk) => {
                        match chunk {
                            Some(chunk) => {
                                // TODO: add warning, write after EOF
                                self.encoder.encode(&mut self.buffer, chunk.as_ref());
                            }
                            None => {
                                // TODO: add error "not eof""
                                if !self.encoder.encode(&mut self.buffer, [].as_ref()) {
                                    debug!("last payload item, but it is not EOF ");
                                    return Err(())
                                }
                                break
                            }
                        }
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
            if self.buffer.len() > DEFAULT_LIMIT {
                self.state = TaskRunningState::Paused;
            } else if self.state == TaskRunningState::Paused {
                self.state = TaskRunningState::Running;
            }
        }

        // response is completed
        if self.buffer.is_empty() && self.iostate.is_done() {
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
                                if msg.body().has_body() {
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

use std::{cmp, io};
use std::fmt::Write;
use bytes::BytesMut;
use futures::{Async, Poll};
use tokio_io::AsyncWrite;
use http::{Version, StatusCode};
use http::header::{HeaderValue,
                   CONNECTION, CONTENT_TYPE, CONTENT_LENGTH, TRANSFER_ENCODING, DATE};

use date;
use body::Body;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;

const AVERAGE_HEADER_SIZE: usize = 30; // totally scientific
const MAX_WRITE_BUFFER_SIZE: usize = 65_536; // max buffer size 64k


#[derive(Debug)]
pub(crate) enum WriterState {
    Done,
    Pause,
}

/// Send stream
pub(crate) trait Writer {
    fn start(&mut self, req: &mut HttpRequest, resp: &mut HttpResponse)
             -> Result<WriterState, io::Error>;

    fn write(&mut self, payload: &[u8]) -> Result<WriterState, io::Error>;

    fn write_eof(&mut self) -> Result<WriterState, io::Error>;

    fn poll_complete(&mut self) -> Poll<(), io::Error>;
}


pub(crate) struct H1Writer<T: AsyncWrite> {
    stream: Option<T>,
    buffer: BytesMut,
    started: bool,
    encoder: Encoder,
    upgrade: bool,
    keepalive: bool,
    disconnected: bool,
}

impl<T: AsyncWrite> H1Writer<T> {

    pub fn new(stream: T) -> H1Writer<T> {
        H1Writer {
            stream: Some(stream),
            buffer: BytesMut::new(),
            started: false,
            encoder: Encoder::length(0),
            upgrade: false,
            keepalive: false,
            disconnected: false,
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.stream.as_mut().unwrap()
    }

    pub fn unwrap(&mut self) -> T {
        self.stream.take().unwrap()
    }

    pub fn disconnected(&mut self) {
        let len = self.buffer.len();
        self.buffer.split_to(len);
    }

    pub fn keepalive(&self) -> bool {
        self.keepalive && !self.upgrade
    }

    fn write_to_stream(&mut self) -> Result<WriterState, io::Error> {
        if let Some(ref mut stream) = self.stream {
            while !self.buffer.is_empty() {
                match stream.write(self.buffer.as_ref()) {
                    Ok(n) => {
                        self.buffer.split_to(n);
                    },
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
                            return Ok(WriterState::Pause)
                        } else {
                            return Ok(WriterState::Done)
                        }
                    }
                    Err(err) => return Err(err),
                }
            }
        }
        Ok(WriterState::Done)
    }
}

impl<T: AsyncWrite> Writer for H1Writer<T> {

    fn start(&mut self, req: &mut HttpRequest, msg: &mut HttpResponse)
             -> Result<WriterState, io::Error>
    {
        trace!("Prepare message status={:?}", msg.status);

        // prepare task
        let mut extra = 0;
        let body = msg.replace_body(Body::Empty);
        let version = msg.version().unwrap_or_else(|| req.version());
        self.started = true;
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
            self.buffer.extend_from_slice(bytes.as_ref());
            return Ok(WriterState::Done)
        }
        msg.replace_body(body);

        Ok(WriterState::Done)
    }

    fn write(&mut self, payload: &[u8]) -> Result<WriterState, io::Error> {
        if !self.disconnected {
            if self.started {
                // TODO: add warning, write after EOF
                self.encoder.encode(&mut self.buffer, payload);
            } else {
                // might be response for EXCEPT
                self.buffer.extend_from_slice(payload)
            }
        }

        if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn write_eof(&mut self) -> Result<WriterState, io::Error> {
        if !self.encoder.encode_eof(&mut self.buffer) {
            //debug!("last payload item, but it is not EOF ");
            Err(io::Error::new(io::ErrorKind::Other,
                               "Last payload item, but eof is not reached"))
        } else if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn poll_complete(&mut self) -> Poll<(), io::Error> {
        match self.write_to_stream() {
            Ok(WriterState::Done) => Ok(Async::Ready(())),
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err)
        }
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug, Clone)]
pub(crate) struct Encoder {
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

    /// Encode eof. Return `EOF` state of encoder
    pub fn encode_eof(&mut self, dst: &mut BytesMut) -> bool {
        match self.kind {
            Kind::Eof => true,
            Kind::Chunked(ref mut eof) => {
                if *eof {
                    return true;
                }

                *eof = true;
                dst.extend(b"0\r\n\r\n");
                true
            },
            Kind::Length(ref mut remaining) => {
                *remaining == 0
            },
        }
    }
}

use std::io;
use futures::{Async, Poll};
use tokio_io::AsyncWrite;
use http::Version;
use http::header::{HeaderValue, CONNECTION, CONTENT_TYPE, DATE};

use date;
use body::Body;
use encoding::PayloadEncoder;
use httprequest::HttpMessage;
use httpresponse::HttpResponse;

const AVERAGE_HEADER_SIZE: usize = 30; // totally scientific
const MAX_WRITE_BUFFER_SIZE: usize = 65_536; // max buffer size 64k


#[derive(Debug)]
pub enum WriterState {
    Done,
    Pause,
}

/// Send stream
pub trait Writer {
    fn written(&self) -> u64;

    fn start(&mut self, req: &mut HttpMessage, resp: &mut HttpResponse)
             -> Result<WriterState, io::Error>;

    fn write(&mut self, payload: &[u8]) -> Result<WriterState, io::Error>;

    fn write_eof(&mut self) -> Result<WriterState, io::Error>;

    fn poll_complete(&mut self) -> Poll<(), io::Error>;
}

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const UPGRADE = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const DISCONNECTED = 0b0000_1000;
    }
}

pub(crate) struct H1Writer<T: AsyncWrite> {
    flags: Flags,
    stream: Option<T>,
    encoder: PayloadEncoder,
    written: u64,
    headers_size: u32,
}

impl<T: AsyncWrite> H1Writer<T> {

    pub fn new(stream: T) -> H1Writer<T> {
        H1Writer {
            flags: Flags::empty(),
            stream: Some(stream),
            encoder: PayloadEncoder::default(),
            written: 0,
            headers_size: 0,
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.stream.as_mut().unwrap()
    }

    pub fn unwrap(&mut self) -> T {
        self.stream.take().unwrap()
    }

    pub fn disconnected(&mut self) {
        self.encoder.get_mut().take();
    }

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE) && !self.flags.contains(Flags::UPGRADE)
    }

    fn write_to_stream(&mut self) -> Result<WriterState, io::Error> {
        let buffer = self.encoder.get_mut();

        if let Some(ref mut stream) = self.stream {
            while !buffer.is_empty() {
                match stream.write(buffer.as_ref()) {
                    Ok(n) => {
                        buffer.split_to(n);
                        self.written += n as u64;
                    },
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if buffer.len() > MAX_WRITE_BUFFER_SIZE {
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

    #[cfg_attr(feature = "cargo-clippy", allow(cast_lossless))]
    fn written(&self) -> u64 {
        if self.written > self.headers_size as u64 {
            self.written - self.headers_size as u64
        } else {
            0
        }
    }

    fn start(&mut self, req: &mut HttpMessage, msg: &mut HttpResponse)
             -> Result<WriterState, io::Error>
    {
        trace!("Prepare response with status: {:?}", msg.status());

        // prepare task
        self.flags.insert(Flags::STARTED);
        self.encoder = PayloadEncoder::new(req, msg);
        if msg.keep_alive().unwrap_or_else(|| req.keep_alive()) {
            self.flags.insert(Flags::KEEPALIVE);
        }

        // Connection upgrade
        let version = msg.version().unwrap_or_else(|| req.version);
        if msg.upgrade() {
            msg.headers_mut().insert(CONNECTION, HeaderValue::from_static("upgrade"));
        }
        // keep-alive
        else if self.flags.contains(Flags::KEEPALIVE) {
            if version < Version::HTTP_11 {
                msg.headers_mut().insert(CONNECTION, HeaderValue::from_static("keep-alive"));
            }
        } else if version >= Version::HTTP_11 {
            msg.headers_mut().insert(CONNECTION, HeaderValue::from_static("close"));
        }

        // render message
        {
            let buffer = self.encoder.get_mut();
            if let Body::Binary(ref bytes) = *msg.body() {
                buffer.reserve(130 + msg.headers().len() * AVERAGE_HEADER_SIZE + bytes.len());
            } else {
                buffer.reserve(130 + msg.headers().len() * AVERAGE_HEADER_SIZE);
            }

            match version {
                Version::HTTP_11 => buffer.extend(b"HTTP/1.1 "),
                Version::HTTP_2 => buffer.extend(b"HTTP/2.0 "),
                Version::HTTP_10 => buffer.extend(b"HTTP/1.0 "),
                Version::HTTP_09 => buffer.extend(b"HTTP/0.9 "),
            }
            buffer.extend(msg.status().as_u16().to_string().as_bytes());
            buffer.extend(b" ");
            buffer.extend(msg.reason().as_bytes());
            buffer.extend(b"\r\n");

            for (key, value) in msg.headers() {
                let t: &[u8] = key.as_ref();
                buffer.extend(t);
                buffer.extend(b": ");
                buffer.extend(value.as_ref());
                buffer.extend(b"\r\n");
            }

            // using http::h1::date is quite a lot faster than generating
            // a unique Date header each time like req/s goes up about 10%
            if !msg.headers().contains_key(DATE) {
                buffer.reserve(date::DATE_VALUE_LENGTH + 8);
                buffer.extend(b"Date: ");
                let mut bytes = [0u8; 29];
                date::extend(&mut bytes[..]);
                buffer.extend(&bytes);
                buffer.extend(b"\r\n");
            }

            // default content-type
            if !msg.headers().contains_key(CONTENT_TYPE) {
                buffer.extend(b"ContentType: application/octet-stream\r\n".as_ref());
            }

            // msg eof
            buffer.extend(b"\r\n");
            self.headers_size = buffer.len() as u32;
        }

        trace!("Response: {:?}", msg);

        if msg.body().is_binary() {
            let body = msg.replace_body(Body::Empty);
            if let Body::Binary(bytes) = body {
                self.encoder.write(bytes.as_ref())?;
                return Ok(WriterState::Done)
            }
        }
        Ok(WriterState::Done)
    }

    fn write(&mut self, payload: &[u8]) -> Result<WriterState, io::Error> {
        if !self.flags.contains(Flags::DISCONNECTED) {
            if self.flags.contains(Flags::STARTED) {
                // TODO: add warning, write after EOF
                self.encoder.write(payload)?;
            } else {
                // might be response to EXCEPT
                self.encoder.get_mut().extend_from_slice(payload)
            }
        }

        if self.encoder.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn write_eof(&mut self) -> Result<WriterState, io::Error> {
        self.encoder.write_eof()?;

        if !self.encoder.is_eof() {
            //debug!("last payload item, but it is not EOF ");
            Err(io::Error::new(io::ErrorKind::Other,
                               "Last payload item, but eof is not reached"))
        } else if self.encoder.len() > MAX_WRITE_BUFFER_SIZE {
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

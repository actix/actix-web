use std::io;
use bytes::BufMut;
use futures::{Async, Poll};
use tokio_io::AsyncWrite;
use http::{Method, Version};
use http::header::{HeaderValue, CONNECTION, DATE};

use helpers;
use body::{Body, Binary};
use httprequest::HttpMessage;
use httpresponse::HttpResponse;
use super::{Writer, WriterState, MAX_WRITE_BUFFER_SIZE};
use super::shared::SharedBytes;
use super::encoding::PayloadEncoder;

const AVERAGE_HEADER_SIZE: usize = 30; // totally scientific

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
    stream: T,
    encoder: PayloadEncoder,
    written: u64,
    headers_size: u32,
    buffer: SharedBytes,
}

impl<T: AsyncWrite> H1Writer<T> {

    pub fn new(stream: T, buf: SharedBytes) -> H1Writer<T> {
        H1Writer {
            flags: Flags::empty(),
            stream: stream,
            encoder: PayloadEncoder::empty(buf.clone()),
            written: 0,
            headers_size: 0,
            buffer: buf,
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.stream
    }

    pub fn reset(&mut self) {
        self.written = 0;
        self.flags = Flags::empty();
    }

    pub fn disconnected(&mut self) {
        self.buffer.take();
    }

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE) && !self.flags.contains(Flags::UPGRADE)
    }

    fn write_to_stream(&mut self) -> io::Result<WriterState> {
        while !self.buffer.is_empty() {
            match self.stream.write(self.buffer.as_ref()) {
                Ok(0) => {
                    self.disconnected();
                    return Ok(WriterState::Done);
                },
                Ok(n) => {
                    let _ = self.buffer.split_to(n);
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
        Ok(WriterState::Done)
    }
}

impl<T: AsyncWrite> Writer for H1Writer<T> {

    #[inline]
    fn written(&self) -> u64 {
        self.written
    }

    fn start(&mut self, req: &mut HttpMessage, msg: &mut HttpResponse) -> io::Result<WriterState> {
        // prepare task
        self.flags.insert(Flags::STARTED);
        self.encoder = PayloadEncoder::new(self.buffer.clone(), req, msg);
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
        let body = msg.replace_body(Body::Empty);

        // render message
        {
            let mut buffer = self.buffer.get_mut();
            if let Body::Binary(ref bytes) = body {
                buffer.reserve(256 + msg.headers().len() * AVERAGE_HEADER_SIZE + bytes.len());
            } else {
                buffer.reserve(256 + msg.headers().len() * AVERAGE_HEADER_SIZE);
            }

            // status line
            helpers::write_status_line(version, msg.status().as_u16(), &mut buffer);
            buffer.extend_from_slice(msg.reason().as_bytes());

            match body {
                Body::Empty =>
                    if req.method != Method::HEAD {
                        buffer.extend_from_slice(b"\r\ncontent-length: 0\r\n");
                    } else {
                        buffer.extend_from_slice(b"\r\n");
                    },
                Body::Binary(ref bytes) =>
                    helpers::write_content_length(bytes.len(), &mut buffer),
                _ =>
                    buffer.extend_from_slice(b"\r\n"),
            }

            // write headers
            for (key, value) in msg.headers() {
                let v = value.as_ref();
                let k = key.as_str().as_bytes();
                buffer.reserve(k.len() + v.len() + 4);
                buffer.put_slice(k);
                buffer.put_slice(b": ");
                buffer.put_slice(v);
                buffer.put_slice(b"\r\n");
            }

            // using helpers::date is quite a lot faster
            if !msg.headers().contains_key(DATE) {
                helpers::date(&mut buffer);
            } else {
                // msg eof
                buffer.extend_from_slice(b"\r\n");
            }
            self.headers_size = buffer.len() as u32;
        }

        if let Body::Binary(bytes) = body {
            self.written = bytes.len() as u64;
            self.encoder.write(bytes)?;
        } else {
            msg.replace_body(body);
        }
        Ok(WriterState::Done)
    }

    fn write(&mut self, payload: Binary) -> io::Result<WriterState> {
        self.written += payload.len() as u64;
        if !self.flags.contains(Flags::DISCONNECTED) {
            if self.flags.contains(Flags::STARTED) {
                // TODO: add warning, write after EOF
                self.encoder.write(payload)?;
                return Ok(WriterState::Done)
            } else {
                // might be response to EXCEPT
                self.buffer.extend_from_slice(payload.as_ref())
            }
        }

        if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn write_eof(&mut self) -> io::Result<WriterState> {
        self.encoder.write_eof()?;

        if !self.encoder.is_eof() {
            Err(io::Error::new(io::ErrorKind::Other,
                               "Last payload item, but eof is not reached"))
        } else if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    #[inline]
    fn poll_completed(&mut self, shutdown: bool) -> Poll<(), io::Error> {
        match self.write_to_stream() {
            Ok(WriterState::Done) => {
                if shutdown {
                    self.stream.shutdown()
                } else {
                    Ok(Async::Ready(()))
                }
            },
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err)
        }
    }
}

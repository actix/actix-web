// #![cfg_attr(feature = "cargo-clippy", allow(redundant_field_names))]

use std::io::{self, Write};
use std::rc::Rc;

use bytes::{BufMut, BytesMut};
use futures::{Async, Poll};
use tokio_io::AsyncWrite;

use super::helpers;
use super::output::{Output, ResponseInfo, ResponseLength};
use super::settings::WorkerSettings;
use super::Request;
use super::{Writer, WriterState, MAX_WRITE_BUFFER_SIZE};
use body::{Binary, Body};
use header::ContentEncoding;
use http::header::{
    HeaderValue, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, DATE, TRANSFER_ENCODING,
};
use http::{Method, Version};
use httpresponse::HttpResponse;

const AVERAGE_HEADER_SIZE: usize = 30; // totally scientific

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const UPGRADE = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const DISCONNECTED = 0b0000_1000;
    }
}

pub(crate) struct H1Writer<T: AsyncWrite, H: 'static> {
    flags: Flags,
    stream: T,
    written: u64,
    headers_size: u32,
    buffer: Output,
    buffer_capacity: usize,
    settings: Rc<WorkerSettings<H>>,
}

impl<T: AsyncWrite, H: 'static> H1Writer<T, H> {
    pub fn new(stream: T, settings: Rc<WorkerSettings<H>>) -> H1Writer<T, H> {
        H1Writer {
            flags: Flags::KEEPALIVE,
            written: 0,
            headers_size: 0,
            buffer: Output::Buffer(settings.get_bytes()),
            buffer_capacity: 0,
            stream,
            settings,
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.stream
    }

    pub fn reset(&mut self) {
        self.written = 0;
        self.flags = Flags::KEEPALIVE;
    }

    pub fn disconnected(&mut self) {}

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE) && !self.flags.contains(Flags::UPGRADE)
    }

    fn write_data(stream: &mut T, data: &[u8]) -> io::Result<usize> {
        let mut written = 0;
        while written < data.len() {
            match stream.write(&data[written..]) {
                Ok(0) => {
                    return Err(io::Error::new(io::ErrorKind::WriteZero, ""));
                }
                Ok(n) => {
                    written += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(written)
                }
                Err(err) => return Err(err),
            }
        }
        Ok(written)
    }
}

impl<T: AsyncWrite, H: 'static> Drop for H1Writer<T, H> {
    fn drop(&mut self) {
        if let Some(bytes) = self.buffer.take_option() {
            self.settings.release_bytes(bytes);
        }
    }
}

impl<T: AsyncWrite, H: 'static> Writer for H1Writer<T, H> {
    #[inline]
    fn written(&self) -> u64 {
        self.written
    }

    #[inline]
    fn set_date(&mut self) {
        self.settings.set_date(self.buffer.as_mut(), true)
    }

    #[inline]
    fn buffer(&mut self) -> &mut BytesMut {
        self.buffer.as_mut()
    }

    fn start(
        &mut self, req: &Request, msg: &mut HttpResponse, encoding: ContentEncoding,
    ) -> io::Result<WriterState> {
        // prepare task
        let mut info = ResponseInfo::new(req.inner.method == Method::HEAD);
        self.buffer.for_server(&mut info, &req.inner, msg, encoding);
        if msg.keep_alive().unwrap_or_else(|| req.keep_alive()) {
            self.flags = Flags::STARTED | Flags::KEEPALIVE;
        } else {
            self.flags = Flags::STARTED;
        }

        // Connection upgrade
        let version = msg.version().unwrap_or_else(|| req.inner.version);
        if msg.upgrade() {
            self.flags.insert(Flags::UPGRADE);
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("upgrade"));
        }
        // keep-alive
        else if self.flags.contains(Flags::KEEPALIVE) {
            if version < Version::HTTP_11 {
                msg.headers_mut()
                    .insert(CONNECTION, HeaderValue::from_static("keep-alive"));
            }
        } else if version >= Version::HTTP_11 {
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("close"));
        }
        let body = msg.replace_body(Body::Empty);

        // render message
        {
            // output buffer
            let mut buffer = self.buffer.as_mut();

            let reason = msg.reason().as_bytes();
            if let Body::Binary(ref bytes) = body {
                buffer.reserve(
                    256
                        + msg.headers().len() * AVERAGE_HEADER_SIZE
                        + bytes.len()
                        + reason.len(),
                );
            } else {
                buffer.reserve(
                    256 + msg.headers().len() * AVERAGE_HEADER_SIZE + reason.len(),
                );
            }

            // status line
            helpers::write_status_line(version, msg.status().as_u16(), &mut buffer);
            buffer.extend_from_slice(reason);

            // content length
            match info.length {
                ResponseLength::Chunked => {
                    buffer.extend_from_slice(b"\r\ntransfer-encoding: chunked\r\n")
                }
                ResponseLength::Zero => {
                    buffer.extend_from_slice(b"\r\ncontent-length: 0\r\n")
                }
                ResponseLength::Length(len) => {
                    helpers::write_content_length(len, &mut buffer)
                }
                ResponseLength::Length64(len) => {
                    buffer.extend_from_slice(b"\r\ncontent-length: ");
                    write!(buffer.writer(), "{}", len)?;
                    buffer.extend_from_slice(b"\r\n");
                }
                ResponseLength::None => buffer.extend_from_slice(b"\r\n"),
            }
            if let Some(ce) = info.content_encoding {
                buffer.extend_from_slice(b"content-encoding: ");
                buffer.extend_from_slice(ce.as_ref());
                buffer.extend_from_slice(b"\r\n");
            }

            // write headers
            let mut pos = 0;
            let mut has_date = false;
            let mut remaining = buffer.remaining_mut();
            unsafe {
                let mut buf = &mut *(buffer.bytes_mut() as *mut [u8]);
                for (key, value) in msg.headers() {
                    match *key {
                        TRANSFER_ENCODING => continue,
                        CONTENT_ENCODING => if encoding != ContentEncoding::Identity {
                            continue;
                        },
                        CONTENT_LENGTH => match info.length {
                            ResponseLength::None => (),
                            _ => continue,
                        },
                        DATE => {
                            has_date = true;
                        }
                        _ => (),
                    }

                    let v = value.as_ref();
                    let k = key.as_str().as_bytes();
                    let len = k.len() + v.len() + 4;
                    if len > remaining {
                        buffer.advance_mut(pos);
                        pos = 0;
                        buffer.reserve(len);
                        remaining = buffer.remaining_mut();
                        buf = &mut *(buffer.bytes_mut() as *mut _);
                    }

                    buf[pos..pos + k.len()].copy_from_slice(k);
                    pos += k.len();
                    buf[pos..pos + 2].copy_from_slice(b": ");
                    pos += 2;
                    buf[pos..pos + v.len()].copy_from_slice(v);
                    pos += v.len();
                    buf[pos..pos + 2].copy_from_slice(b"\r\n");
                    pos += 2;
                    remaining -= len;
                }
                buffer.advance_mut(pos);
            }

            // optimized date header, set_date writes \r\n
            if !has_date {
                self.settings.set_date(&mut buffer, true);
            } else {
                // msg eof
                buffer.extend_from_slice(b"\r\n");
            }
            self.headers_size = buffer.len() as u32;
        }

        if let Body::Binary(bytes) = body {
            self.written = bytes.len() as u64;
            self.buffer.write(bytes.as_ref())?;
        } else {
            // capacity, makes sense only for streaming or actor
            self.buffer_capacity = msg.write_buffer_capacity();

            msg.replace_body(body);
        }
        Ok(WriterState::Done)
    }

    fn write(&mut self, payload: &Binary) -> io::Result<WriterState> {
        self.written += payload.len() as u64;
        if !self.flags.contains(Flags::DISCONNECTED) {
            if self.flags.contains(Flags::STARTED) {
                // shortcut for upgraded connection
                if self.flags.contains(Flags::UPGRADE) {
                    if self.buffer.is_empty() {
                        let pl: &[u8] = payload.as_ref();
                        let n = match Self::write_data(&mut self.stream, pl) {
                            Err(err) => {
                                if err.kind() == io::ErrorKind::WriteZero {
                                    self.disconnected();
                                }

                                return Err(err);
                            }
                            Ok(val) => val,
                        };
                        if n < pl.len() {
                            self.buffer.write(&pl[n..])?;
                            return Ok(WriterState::Done);
                        }
                    } else {
                        self.buffer.write(payload.as_ref())?;
                    }
                } else {
                    // TODO: add warning, write after EOF
                    self.buffer.write(payload.as_ref())?;
                }
            } else {
                // could be response to EXCEPT header
                self.buffer.write(payload.as_ref())?;
            }
        }

        if self.buffer.len() > self.buffer_capacity {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    fn write_eof(&mut self) -> io::Result<WriterState> {
        if !self.buffer.write_eof()? {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "Last payload item, but eof is not reached",
            ))
        } else if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    #[inline]
    fn poll_completed(&mut self, shutdown: bool) -> Poll<(), io::Error> {
        if !self.buffer.is_empty() {
            let written = {
                match Self::write_data(&mut self.stream, self.buffer.as_ref().as_ref()) {
                    Err(err) => {
                        if err.kind() == io::ErrorKind::WriteZero {
                            self.disconnected();
                        }

                        return Err(err);
                    }
                    Ok(val) => val,
                }
            };
            let _ = self.buffer.split_to(written);
            if shutdown && !self.buffer.is_empty()
                || (self.buffer.len() > self.buffer_capacity)
            {
                return Ok(Async::NotReady);
            }
        }
        if shutdown {
            self.stream.poll_flush()?;
            self.stream.shutdown()
        } else {
            self.stream.poll_flush()
        }
    }
}

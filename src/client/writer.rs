#![allow(dead_code)]
use std::io;
use std::fmt::Write;
use bytes::BufMut;
use futures::{Async, Poll};
use tokio_io::AsyncWrite;

use body::Binary;
use server::WriterState;
use server::shared::SharedBytes;

use client::ClientRequest;


const LOW_WATERMARK: usize = 1024;
const HIGH_WATERMARK: usize = 8 * LOW_WATERMARK;
const AVERAGE_HEADER_SIZE: usize = 30;

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const UPGRADE = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const DISCONNECTED = 0b0000_1000;
    }
}

pub(crate) struct HttpClientWriter {
    flags: Flags,
    written: u64,
    headers_size: u32,
    buffer: SharedBytes,
    low: usize,
    high: usize,
}

impl HttpClientWriter {

    pub fn new(buf: SharedBytes) -> HttpClientWriter {
        HttpClientWriter {
            flags: Flags::empty(),
            written: 0,
            headers_size: 0,
            buffer: buf,
            low: LOW_WATERMARK,
            high: HIGH_WATERMARK,
        }
    }

    pub fn disconnected(&mut self) {
        self.buffer.take();
    }

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE) && !self.flags.contains(Flags::UPGRADE)
    }

    /// Set write buffer capacity
    pub fn set_buffer_capacity(&mut self, low_watermark: usize, high_watermark: usize) {
        self.low = low_watermark;
        self.high = high_watermark;
    }

    fn write_to_stream<T: AsyncWrite>(&mut self, stream: &mut T) -> io::Result<WriterState> {
        while !self.buffer.is_empty() {
            match stream.write(self.buffer.as_ref()) {
                Ok(0) => {
                    self.disconnected();
                    return Ok(WriterState::Done);
                },
                Ok(n) => {
                    let _ = self.buffer.split_to(n);
                },
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if self.buffer.len() > self.high {
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

impl HttpClientWriter {

    pub fn start(&mut self, msg: &mut ClientRequest) {
        // prepare task
        self.flags.insert(Flags::STARTED);

        // render message
        {
            let buffer = self.buffer.get_mut();
            buffer.reserve(256 + msg.headers().len() * AVERAGE_HEADER_SIZE);

            // status line
            let _ = write!(buffer, "{} {} {:?}\r\n",
                   msg.method(), msg.uri().path(), msg.version());

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
            //if !msg.headers.contains_key(DATE) {
            //    helpers::date(&mut buffer);
            //} else {
                // msg eof
                buffer.extend_from_slice(b"\r\n");
            //}
            self.headers_size = buffer.len() as u32;
        }
    }

    pub fn write(&mut self, payload: &Binary) -> io::Result<WriterState> {
        self.written += payload.len() as u64;
        if !self.flags.contains(Flags::DISCONNECTED) {
            self.buffer.extend_from_slice(payload.as_ref())
        }

        if self.buffer.len() > self.high {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    pub fn write_eof(&mut self) -> io::Result<WriterState> {
        if self.buffer.len() > self.high {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    #[inline]
    pub fn poll_completed<T: AsyncWrite>(&mut self, stream: &mut T, shutdown: bool)
                                         -> Poll<(), io::Error>
    {
        match self.write_to_stream(stream) {
            Ok(WriterState::Done) => {
                if shutdown {
                    stream.shutdown()
                } else {
                    Ok(Async::Ready(()))
                }
            },
            Ok(WriterState::Pause) => Ok(Async::NotReady),
            Err(err) => Err(err)
        }
    }
}

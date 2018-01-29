#![allow(dead_code)]
use std::io;
use bytes::BufMut;
use futures::{Async, Poll};
use tokio_io::AsyncWrite;
// use http::header::{HeaderValue, CONNECTION, DATE};

use body::Binary;
use server::{WriterState, MAX_WRITE_BUFFER_SIZE};
use server::shared::SharedBytes;

use client::ClientRequest;


const AVERAGE_HEADER_SIZE: usize = 30; // totally scientific

bitflags! {
    struct Flags: u8 {
        const STARTED = 0b0000_0001;
        const UPGRADE = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const DISCONNECTED = 0b0000_1000;
    }
}

pub(crate) struct Writer {
    flags: Flags,
    written: u64,
    headers_size: u32,
    buffer: SharedBytes,
}

impl Writer {

    pub fn new(buf: SharedBytes) -> Writer {
        Writer {
            flags: Flags::empty(),
            written: 0,
            headers_size: 0,
            buffer: buf,
        }
    }

    pub fn disconnected(&mut self) {
        self.buffer.take();
    }

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE) && !self.flags.contains(Flags::UPGRADE)
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

impl Writer {

    pub fn start(&mut self, msg: &mut ClientRequest) {
        // prepare task
        self.flags.insert(Flags::STARTED);

        // render message
        {
            let buffer = self.buffer.get_mut();
            buffer.reserve(256 + msg.headers().len() * AVERAGE_HEADER_SIZE);

            // status line
            // helpers::write_status_line(version, msg.status().as_u16(), &mut buffer);
            // buffer.extend_from_slice(msg.reason().as_bytes());
            buffer.extend_from_slice(b"GET ");
            buffer.extend_from_slice(msg.uri().path().as_ref());
            buffer.extend_from_slice(b" HTTP/1.1\r\n");

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

        if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
            Ok(WriterState::Pause)
        } else {
            Ok(WriterState::Done)
        }
    }

    pub fn write_eof(&mut self) -> io::Result<WriterState> {
        if self.buffer.len() > MAX_WRITE_BUFFER_SIZE {
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

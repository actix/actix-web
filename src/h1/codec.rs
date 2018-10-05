#![allow(unused_imports, unused_variables, dead_code)]
use std::io::{self, Write};

use bytes::{BufMut, Bytes, BytesMut};
use tokio_codec::{Decoder, Encoder};

use super::decoder::H1Decoder;
pub use super::decoder::InMessage;
use super::encoder::{ResponseEncoder, ResponseLength};
use body::Body;
use error::ParseError;
use helpers;
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use http::{Method, Version};
use request::RequestPool;
use response::Response;

bitflags! {
    struct Flags: u8 {
        const HEAD = 0b0000_0001;
        const UPGRADE = 0b0000_0010;
        const KEEPALIVE = 0b0000_0100;
        const KEEPALIVE_ENABLED = 0b0001_0000;
    }
}

const AVERAGE_HEADER_SIZE: usize = 30;

/// Http response
pub enum OutMessage {
    /// Http response message
    Response(Response),
    /// Payload chunk
    Payload(Bytes),
}

/// HTTP/1 Codec
pub struct Codec {
    decoder: H1Decoder,
    version: Version,

    // encoder part
    flags: Flags,
    written: u64,
    headers_size: u32,
    te: ResponseEncoder,
}

impl Codec {
    /// Create HTTP/1 codec.
    ///
    /// `keepalive_enabled` how response `connection` header get generated.
    pub fn new(keepalive_enabled: bool) -> Self {
        Codec::with_pool(RequestPool::pool(), keepalive_enabled)
    }

    /// Create HTTP/1 codec with request's pool
    pub(crate) fn with_pool(
        pool: &'static RequestPool, keepalive_enabled: bool,
    ) -> Self {
        let flags = if keepalive_enabled {
            Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };
        Codec {
            decoder: H1Decoder::with_pool(pool),
            version: Version::HTTP_11,

            flags,
            written: 0,
            headers_size: 0,
            te: ResponseEncoder::default(),
        }
    }

    fn written(&self) -> u64 {
        self.written
    }

    pub fn upgrade(&self) -> bool {
        self.flags.contains(Flags::UPGRADE)
    }

    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE)
    }

    fn encode_response(
        &mut self, mut msg: Response, buffer: &mut BytesMut,
    ) -> io::Result<()> {
        // prepare transfer encoding
        self.te
            .update(&mut msg, self.flags.contains(Flags::HEAD), self.version);

        let ka = self.flags.contains(Flags::KEEPALIVE_ENABLED) && msg
            .keep_alive()
            .unwrap_or_else(|| self.flags.contains(Flags::KEEPALIVE));

        // Connection upgrade
        let version = msg.version().unwrap_or_else(|| self.version);
        if msg.upgrade() {
            self.flags.insert(Flags::UPGRADE);
            self.flags.remove(Flags::KEEPALIVE);
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("upgrade"));
        }
        // keep-alive
        else if ka {
            self.flags.insert(Flags::KEEPALIVE);
            if version < Version::HTTP_11 {
                msg.headers_mut()
                    .insert(CONNECTION, HeaderValue::from_static("keep-alive"));
            }
        } else if version >= Version::HTTP_11 {
            self.flags.remove(Flags::KEEPALIVE);
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("close"));
        }
        let body = msg.replace_body(Body::Empty);

        // render message
        {
            let reason = msg.reason().as_bytes();
            if let Body::Binary(ref bytes) = body {
                buffer.reserve(
                    256 + msg.headers().len() * AVERAGE_HEADER_SIZE
                        + bytes.len()
                        + reason.len(),
                );
            } else {
                buffer.reserve(
                    256 + msg.headers().len() * AVERAGE_HEADER_SIZE + reason.len(),
                );
            }

            // status line
            helpers::write_status_line(version, msg.status().as_u16(), buffer);
            buffer.extend_from_slice(reason);

            // content length
            match self.te.length {
                ResponseLength::Chunked => {
                    buffer.extend_from_slice(b"\r\ntransfer-encoding: chunked\r\n")
                }
                ResponseLength::Zero => {
                    buffer.extend_from_slice(b"\r\ncontent-length: 0\r\n")
                }
                ResponseLength::Length(len) => {
                    helpers::write_content_length(len, buffer)
                }
                ResponseLength::Length64(len) => {
                    buffer.extend_from_slice(b"\r\ncontent-length: ");
                    write!(buffer.writer(), "{}", len)?;
                    buffer.extend_from_slice(b"\r\n");
                }
                ResponseLength::None => buffer.extend_from_slice(b"\r\n"),
            }

            // write headers
            let mut pos = 0;
            let mut has_date = false;
            let mut remaining = buffer.remaining_mut();
            let mut buf = unsafe { &mut *(buffer.bytes_mut() as *mut [u8]) };
            for (key, value) in msg.headers() {
                match *key {
                    TRANSFER_ENCODING => continue,
                    CONTENT_LENGTH => match self.te.length {
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
                    unsafe {
                        buffer.advance_mut(pos);
                    }
                    pos = 0;
                    buffer.reserve(len);
                    remaining = buffer.remaining_mut();
                    unsafe {
                        buf = &mut *(buffer.bytes_mut() as *mut _);
                    }
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
            unsafe {
                buffer.advance_mut(pos);
            }

            // optimized date header, set_date writes \r\n
            if !has_date {
                // self.settings.set_date(&mut buffer, true);
                buffer.extend_from_slice(b"\r\n");
            } else {
                // msg eof
                buffer.extend_from_slice(b"\r\n");
            }
            self.headers_size = buffer.len() as u32;
        }

        if let Body::Binary(bytes) = body {
            self.written = bytes.len() as u64;
            // buffer.write(bytes.as_ref())?;
            buffer.extend_from_slice(bytes.as_ref());
        } else {
            // capacity, makes sense only for streaming or actor
            // self.buffer_capacity = msg.write_buffer_capacity();

            msg.replace_body(body);
        }
        Ok(())
    }
}

impl Decoder for Codec {
    type Item = InMessage;
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let res = self.decoder.decode(src);

        match res {
            Ok(Some(InMessage::Message(ref req)))
            | Ok(Some(InMessage::MessageWithPayload(ref req))) => {
                self.flags
                    .set(Flags::HEAD, req.inner.method == Method::HEAD);
                self.version = req.inner.version;
                if self.flags.contains(Flags::KEEPALIVE_ENABLED) {
                    self.flags.set(Flags::KEEPALIVE, req.keep_alive());
                }
            }
            _ => (),
        }
        res
    }
}

impl Encoder for Codec {
    type Item = OutMessage;
    type Error = io::Error;

    fn encode(
        &mut self, item: Self::Item, dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            OutMessage::Response(res) => {
                self.written = 0;
                self.encode_response(res, dst)?;
            }
            OutMessage::Payload(bytes) => {
                dst.extend_from_slice(&bytes);
            }
        }
        Ok(())
    }
}

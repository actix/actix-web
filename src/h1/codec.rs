#![allow(unused_imports, unused_variables, dead_code)]
use std::fmt;
use std::io::{self, Write};

use bytes::{BufMut, Bytes, BytesMut};
use tokio_codec::{Decoder, Encoder};

use super::decoder::{PayloadDecoder, PayloadItem, PayloadType, RequestDecoder};
use super::encoder::{ResponseEncoder, ResponseLength};
use super::{Message, MessageType};
use body::{Binary, Body};
use config::ServiceConfig;
use error::ParseError;
use helpers;
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use http::{Method, Version};
use request::{MessagePool, Request};
use response::Response;

bitflags! {
    struct Flags: u8 {
        const HEAD              = 0b0000_0001;
        const UPGRADE           = 0b0000_0010;
        const KEEPALIVE         = 0b0000_0100;
        const KEEPALIVE_ENABLED = 0b0000_1000;
        const UNHANDLED         = 0b0001_0000;
    }
}

const AVERAGE_HEADER_SIZE: usize = 30;

/// HTTP/1 Codec
pub struct Codec {
    config: ServiceConfig,
    decoder: RequestDecoder,
    payload: Option<PayloadDecoder>,
    version: Version,

    // encoder part
    flags: Flags,
    headers_size: u32,
    te: ResponseEncoder,
}

impl fmt::Debug for Codec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "h1::Codec({:?})", self.flags)
    }
}

impl Codec {
    /// Create HTTP/1 codec.
    ///
    /// `keepalive_enabled` how response `connection` header get generated.
    pub fn new(config: ServiceConfig) -> Self {
        Codec::with_pool(MessagePool::pool(), config)
    }

    /// Create HTTP/1 codec with request's pool
    pub(crate) fn with_pool(pool: &'static MessagePool, config: ServiceConfig) -> Self {
        let flags = if config.keep_alive_enabled() {
            Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };
        Codec {
            config,
            decoder: RequestDecoder::with_pool(pool),
            payload: None,
            version: Version::HTTP_11,

            flags,
            headers_size: 0,
            te: ResponseEncoder::default(),
        }
    }

    /// Check if request is upgrade
    pub fn upgrade(&self) -> bool {
        self.flags.contains(Flags::UPGRADE)
    }

    /// Check if last response is keep-alive
    pub fn keepalive(&self) -> bool {
        self.flags.contains(Flags::KEEPALIVE)
    }

    /// Check last request's message type
    pub fn message_type(&self) -> MessageType {
        if self.flags.contains(Flags::UNHANDLED) {
            MessageType::Unhandled
        } else if self.payload.is_none() {
            MessageType::None
        } else {
            MessageType::Payload
        }
    }

    /// prepare transfer encoding
    pub fn prepare_te(&mut self, res: &mut Response) {
        self.te
            .update(res, self.flags.contains(Flags::HEAD), self.version);
    }

    fn encode_response(
        &mut self, mut msg: Response, buffer: &mut BytesMut,
    ) -> io::Result<()> {
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

        // render message
        {
            let reason = msg.reason().as_bytes();
            if let Body::Binary(ref bytes) = msg.body() {
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
            let mut len_is_set = true;
            match self.te.length {
                ResponseLength::Chunked => {
                    buffer.extend_from_slice(b"\r\ntransfer-encoding: chunked\r\n")
                }
                ResponseLength::Zero => {
                    len_is_set = false;
                    buffer.extend_from_slice(b"\r\n")
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
                        ResponseLength::Zero => {
                            len_is_set = true;
                        }
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
            if !len_is_set {
                buffer.extend_from_slice(b"content-length: 0\r\n")
            }

            // optimized date header, set_date writes \r\n
            if !has_date {
                self.config.set_date(buffer);
            } else {
                // msg eof
                buffer.extend_from_slice(b"\r\n");
            }
            self.headers_size = buffer.len() as u32;
        }

        Ok(())
    }
}

impl Decoder for Codec {
    type Item = Message<Request>;
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.payload.is_some() {
            Ok(match self.payload.as_mut().unwrap().decode(src)? {
                Some(PayloadItem::Chunk(chunk)) => Some(Message::Chunk(Some(chunk))),
                Some(PayloadItem::Eof) => Some(Message::Chunk(None)),
                None => None,
            })
        } else if self.flags.contains(Flags::UNHANDLED) {
            Ok(None)
        } else if let Some((req, payload)) = self.decoder.decode(src)? {
            self.flags
                .set(Flags::HEAD, req.inner.method == Method::HEAD);
            self.version = req.inner.version;
            if self.flags.contains(Flags::KEEPALIVE_ENABLED) {
                self.flags.set(Flags::KEEPALIVE, req.keep_alive());
            }
            match payload {
                PayloadType::None => self.payload = None,
                PayloadType::Payload(pl) => self.payload = Some(pl),
                PayloadType::Unhandled => {
                    self.payload = None;
                    self.flags.insert(Flags::UNHANDLED);
                }
            }
            Ok(Some(Message::Item(req)))
        } else {
            Ok(None)
        }
    }
}

impl Encoder for Codec {
    type Item = Message<Response>;
    type Error = io::Error;

    fn encode(
        &mut self, item: Self::Item, dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            Message::Item(res) => {
                self.encode_response(res, dst)?;
            }
            Message::Chunk(Some(bytes)) => {
                self.te.encode(bytes.as_ref(), dst)?;
            }
            Message::Chunk(None) => {
                self.te.encode_eof(dst)?;
            }
        }
        Ok(())
    }
}

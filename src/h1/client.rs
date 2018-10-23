#![allow(unused_imports, unused_variables, dead_code)]
use std::io::{self, Write};

use bytes::{BufMut, Bytes, BytesMut};
use tokio_codec::{Decoder, Encoder};

use super::decoder::{PayloadDecoder, PayloadItem, PayloadType, ResponseDecoder};
use super::encoder::{RequestEncoder, ResponseLength};
use super::{Message, MessageType};
use body::{Binary, Body};
use client::{ClientRequest, ClientResponse};
use config::ServiceConfig;
use error::ParseError;
use helpers;
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use http::{Method, Version};
use request::MessagePool;

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
pub struct ClientCodec {
    config: ServiceConfig,
    decoder: ResponseDecoder,
    payload: Option<PayloadDecoder>,
    version: Version,

    // encoder part
    flags: Flags,
    headers_size: u32,
    te: RequestEncoder,
}

impl Default for ClientCodec {
    fn default() -> Self {
        ClientCodec::new(ServiceConfig::default())
    }
}

impl ClientCodec {
    /// Create HTTP/1 codec.
    ///
    /// `keepalive_enabled` how response `connection` header get generated.
    pub fn new(config: ServiceConfig) -> Self {
        ClientCodec::with_pool(MessagePool::pool(), config)
    }

    /// Create HTTP/1 codec with request's pool
    pub(crate) fn with_pool(pool: &'static MessagePool, config: ServiceConfig) -> Self {
        let flags = if config.keep_alive_enabled() {
            Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };
        ClientCodec {
            config,
            decoder: ResponseDecoder::with_pool(pool),
            payload: None,
            version: Version::HTTP_11,

            flags,
            headers_size: 0,
            te: RequestEncoder::default(),
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
    pub fn prepare_te(&mut self, res: &mut ClientRequest) {
        self.te
            .update(res, self.flags.contains(Flags::HEAD), self.version);
    }

    fn encode_response(
        &mut self, msg: ClientRequest, buffer: &mut BytesMut,
    ) -> io::Result<()> {
        // Connection upgrade
        if msg.upgrade() {
            self.flags.insert(Flags::UPGRADE);
        }

        // render message
        {
            // status line
            writeln!(
                Writer(buffer),
                "{} {} {:?}\r",
                msg.method(),
                msg.uri()
                    .path_and_query()
                    .map(|u| u.as_str())
                    .unwrap_or("/"),
                msg.version()
            ).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            // write headers
            buffer.reserve(msg.headers().len() * AVERAGE_HEADER_SIZE);
            for (key, value) in msg.headers() {
                let v = value.as_ref();
                let k = key.as_str().as_bytes();
                buffer.reserve(k.len() + v.len() + 4);
                buffer.put_slice(k);
                buffer.put_slice(b": ");
                buffer.put_slice(v);
                buffer.put_slice(b"\r\n");
            }

            // set date header
            if !msg.headers().contains_key(DATE) {
                self.config.set_date(buffer);
            } else {
                buffer.extend_from_slice(b"\r\n");
            }
        }

        Ok(())
    }
}

impl Decoder for ClientCodec {
    type Item = Message<ClientResponse>;
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
            };
            Ok(Some(Message::Item(req)))
        } else {
            Ok(None)
        }
    }
}

impl Encoder for ClientCodec {
    type Item = Message<ClientRequest>;
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

pub struct Writer<'a>(pub &'a mut BytesMut);

impl<'a> io::Write for Writer<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

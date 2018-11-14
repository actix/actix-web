#![allow(unused_imports, unused_variables, dead_code)]
use std::io::{self, Write};

use bytes::{BufMut, Bytes, BytesMut};
use tokio_codec::{Decoder, Encoder};

use super::decoder::{PayloadDecoder, PayloadItem, PayloadType, ResponseDecoder};
use super::encoder::{RequestEncoder, ResponseLength};
use super::{Message, MessageType};
use body::{Binary, Body, BodyType};
use client::{ClientResponse, RequestHead};
use config::ServiceConfig;
use error::{ParseError, PayloadError};
use helpers;
use http::header::{
    HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING, UPGRADE,
};
use http::{Method, Version};
use request::MessagePool;

bitflags! {
    struct Flags: u8 {
        const HEAD              = 0b0000_0001;
        const UPGRADE           = 0b0000_0010;
        const KEEPALIVE         = 0b0000_0100;
        const KEEPALIVE_ENABLED = 0b0000_1000;
        const STREAM            = 0b0001_0000;
    }
}

const AVERAGE_HEADER_SIZE: usize = 30;

/// HTTP/1 Codec
pub struct ClientCodec {
    inner: ClientCodecInner,
}

/// HTTP/1 Payload Codec
pub struct ClientPayloadCodec {
    inner: ClientCodecInner,
}

struct ClientCodecInner {
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
            inner: ClientCodecInner {
                config,
                decoder: ResponseDecoder::with_pool(pool),
                payload: None,
                version: Version::HTTP_11,

                flags,
                headers_size: 0,
                te: RequestEncoder::default(),
            },
        }
    }

    /// Check if request is upgrade
    pub fn upgrade(&self) -> bool {
        self.inner.flags.contains(Flags::UPGRADE)
    }

    /// Check if last response is keep-alive
    pub fn keepalive(&self) -> bool {
        self.inner.flags.contains(Flags::KEEPALIVE)
    }

    /// Check last request's message type
    pub fn message_type(&self) -> MessageType {
        if self.inner.flags.contains(Flags::STREAM) {
            MessageType::Stream
        } else if self.inner.payload.is_none() {
            MessageType::None
        } else {
            MessageType::Payload
        }
    }

    /// prepare transfer encoding
    pub fn prepare_te(&mut self, head: &mut RequestHead, btype: BodyType) {
        self.inner.te.update(
            head,
            self.inner.flags.contains(Flags::HEAD),
            self.inner.version,
        );
    }

    /// Convert message codec to a payload codec
    pub fn into_payload_codec(self) -> ClientPayloadCodec {
        ClientPayloadCodec { inner: self.inner }
    }
}

impl ClientPayloadCodec {
    /// Transform payload codec to a message codec
    pub fn into_message_codec(self) -> ClientCodec {
        ClientCodec { inner: self.inner }
    }
}

impl ClientCodecInner {
    fn encode_response(
        &mut self,
        msg: RequestHead,
        btype: BodyType,
        buffer: &mut BytesMut,
    ) -> io::Result<()> {
        // render message
        {
            // status line
            writeln!(
                Writer(buffer),
                "{} {} {:?}\r",
                msg.method,
                msg.uri.path_and_query().map(|u| u.as_str()).unwrap_or("/"),
                msg.version
            ).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            // write headers
            buffer.reserve(msg.headers.len() * AVERAGE_HEADER_SIZE);
            for (key, value) in &msg.headers {
                let v = value.as_ref();
                let k = key.as_str().as_bytes();
                buffer.reserve(k.len() + v.len() + 4);
                buffer.put_slice(k);
                buffer.put_slice(b": ");
                buffer.put_slice(v);
                buffer.put_slice(b"\r\n");

                // Connection upgrade
                if key == UPGRADE {
                    self.flags.insert(Flags::UPGRADE);
                }
            }

            // set date header
            if !msg.headers.contains_key(DATE) {
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
        if self.inner.payload.is_some() {
            Ok(match self.inner.payload.as_mut().unwrap().decode(src)? {
                Some(PayloadItem::Chunk(chunk)) => Some(Message::Chunk(Some(chunk))),
                Some(PayloadItem::Eof) => Some(Message::Chunk(None)),
                None => None,
            })
        } else if let Some((req, payload)) = self.inner.decoder.decode(src)? {
            self.inner
                .flags
                .set(Flags::HEAD, req.inner.method == Method::HEAD);
            self.inner.version = req.inner.version;
            if self.inner.flags.contains(Flags::KEEPALIVE_ENABLED) {
                self.inner.flags.set(Flags::KEEPALIVE, req.keep_alive());
            }
            match payload {
                PayloadType::None => self.inner.payload = None,
                PayloadType::Payload(pl) => self.inner.payload = Some(pl),
                PayloadType::Stream(pl) => {
                    self.inner.payload = Some(pl);
                    self.inner.flags.insert(Flags::STREAM);
                }
            };
            Ok(Some(Message::Item(req)))
        } else {
            Ok(None)
        }
    }
}

impl Decoder for ClientPayloadCodec {
    type Item = Option<Bytes>;
    type Error = PayloadError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        assert!(
            self.inner.payload.is_some(),
            "Payload decoder is not specified"
        );

        Ok(match self.inner.payload.as_mut().unwrap().decode(src)? {
            Some(PayloadItem::Chunk(chunk)) => Some(Some(chunk)),
            Some(PayloadItem::Eof) => {
                self.inner.payload.take();
                Some(None)
            }
            None => None,
        })
    }
}

impl Encoder for ClientCodec {
    type Item = Message<(RequestHead, BodyType)>;
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            Message::Item((msg, btype)) => {
                self.inner.encode_response(msg, btype, dst)?;
            }
            Message::Chunk(Some(bytes)) => {
                self.inner.te.encode(bytes.as_ref(), dst)?;
            }
            Message::Chunk(None) => {
                self.inner.te.encode_eof(dst)?;
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

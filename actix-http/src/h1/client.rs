#![allow(unused_imports, unused_variables, dead_code)]
use std::io::{self, Write};

use actix_codec::{Decoder, Encoder};
use bitflags::bitflags;
use bytes::{BufMut, Bytes, BytesMut};
use http::header::{
    HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING, UPGRADE,
};
use http::{Method, Version};

use super::decoder::{PayloadDecoder, PayloadItem, PayloadType};
use super::{decoder, encoder, reserve_readbuf};
use super::{Message, MessageType};
use crate::body::BodySize;
use crate::config::ServiceConfig;
use crate::error::{ParseError, PayloadError};
use crate::helpers;
use crate::message::{ConnectionType, Head, MessagePool, RequestHead, ResponseHead};

bitflags! {
    struct Flags: u8 {
        const HEAD              = 0b0000_0001;
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
    decoder: decoder::MessageDecoder<ResponseHead>,
    payload: Option<PayloadDecoder>,
    version: Version,
    ctype: ConnectionType,

    // encoder part
    flags: Flags,
    headers_size: u32,
    encoder: encoder::MessageEncoder<RequestHead>,
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
        let flags = if config.keep_alive_enabled() {
            Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };
        ClientCodec {
            inner: ClientCodecInner {
                config,
                decoder: decoder::MessageDecoder::default(),
                payload: None,
                version: Version::HTTP_11,
                ctype: ConnectionType::Close,

                flags,
                headers_size: 0,
                encoder: encoder::MessageEncoder::default(),
            },
        }
    }

    /// Check if request is upgrade
    pub fn upgrade(&self) -> bool {
        self.inner.ctype == ConnectionType::Upgrade
    }

    /// Check if last response is keep-alive
    pub fn keepalive(&self) -> bool {
        self.inner.ctype == ConnectionType::KeepAlive
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

    /// Convert message codec to a payload codec
    pub fn into_payload_codec(self) -> ClientPayloadCodec {
        ClientPayloadCodec { inner: self.inner }
    }
}

impl ClientPayloadCodec {
    /// Check if last response is keep-alive
    pub fn keepalive(&self) -> bool {
        self.inner.ctype == ConnectionType::KeepAlive
    }

    /// Transform payload codec to a message codec
    pub fn into_message_codec(self) -> ClientCodec {
        ClientCodec { inner: self.inner }
    }
}

impl Decoder for ClientCodec {
    type Item = ResponseHead;
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        debug_assert!(!self.inner.payload.is_some(), "Payload decoder is set");

        if let Some((req, payload)) = self.inner.decoder.decode(src)? {
            if let Some(ctype) = req.ctype() {
                // do not use peer's keep-alive
                self.inner.ctype = if ctype == ConnectionType::KeepAlive {
                    self.inner.ctype
                } else {
                    ctype
                };
            }

            if !self.inner.flags.contains(Flags::HEAD) {
                match payload {
                    PayloadType::None => self.inner.payload = None,
                    PayloadType::Payload(pl) => self.inner.payload = Some(pl),
                    PayloadType::Stream(pl) => {
                        self.inner.payload = Some(pl);
                        self.inner.flags.insert(Flags::STREAM);
                    }
                }
            } else {
                self.inner.payload = None;
            }
            reserve_readbuf(src);
            Ok(Some(req))
        } else {
            Ok(None)
        }
    }
}

impl Decoder for ClientPayloadCodec {
    type Item = Option<Bytes>;
    type Error = PayloadError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        debug_assert!(
            self.inner.payload.is_some(),
            "Payload decoder is not specified"
        );

        Ok(match self.inner.payload.as_mut().unwrap().decode(src)? {
            Some(PayloadItem::Chunk(chunk)) => {
                reserve_readbuf(src);
                Some(Some(chunk))
            }
            Some(PayloadItem::Eof) => {
                self.inner.payload.take();
                Some(None)
            }
            None => None,
        })
    }
}

impl Encoder for ClientCodec {
    type Item = Message<(RequestHead, BodySize)>;
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            Message::Item((mut msg, length)) => {
                let inner = &mut self.inner;
                inner.version = msg.version;
                inner.flags.set(Flags::HEAD, msg.method == Method::HEAD);

                // connection status
                inner.ctype = match msg.connection_type() {
                    ConnectionType::KeepAlive => {
                        if inner.flags.contains(Flags::KEEPALIVE_ENABLED) {
                            ConnectionType::KeepAlive
                        } else {
                            ConnectionType::Close
                        }
                    }
                    ConnectionType::Upgrade => ConnectionType::Upgrade,
                    ConnectionType::Close => ConnectionType::Close,
                };

                inner.encoder.encode(
                    dst,
                    &mut msg,
                    false,
                    false,
                    inner.version,
                    length,
                    inner.ctype,
                    &inner.config,
                )?;
            }
            Message::Chunk(Some(bytes)) => {
                self.inner.encoder.encode_chunk(bytes.as_ref(), dst)?;
            }
            Message::Chunk(None) => {
                self.inner.encoder.encode_eof(dst)?;
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

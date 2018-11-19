#![allow(unused_imports, unused_variables, dead_code)]
use std::io::{self, Write};

use bytes::{BufMut, Bytes, BytesMut};
use tokio_codec::{Decoder, Encoder};

use super::decoder::{MessageDecoder, PayloadDecoder, PayloadItem, PayloadType};
use super::encoder::RequestEncoder;
use super::{Message, MessageType};
use body::BodyLength;
use client::ClientResponse;
use config::ServiceConfig;
use error::{ParseError, PayloadError};
use helpers;
use http::header::{
    HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING, UPGRADE,
};
use http::{Method, Version};
use message::{Head, MessagePool, RequestHead};

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
    decoder: MessageDecoder<ClientResponse>,
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
        let flags = if config.keep_alive_enabled() {
            Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };
        ClientCodec {
            inner: ClientCodecInner {
                config,
                decoder: MessageDecoder::default(),
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
    pub fn prepare_te(&mut self, head: &mut RequestHead, length: BodyLength) {
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

fn prn_version(ver: Version) -> &'static str {
    match ver {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2.0",
    }
}

impl ClientCodecInner {
    fn encode_request(
        &mut self,
        msg: RequestHead,
        length: BodyLength,
        buffer: &mut BytesMut,
    ) -> io::Result<()> {
        // render message
        {
            // status line
            write!(
                Writer(buffer),
                "{} {} {}\r\n",
                msg.method,
                msg.uri.path_and_query().map(|u| u.as_str()).unwrap_or("/"),
                prn_version(msg.version)
            ).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            // write headers
            buffer.reserve(msg.headers.len() * AVERAGE_HEADER_SIZE);

            // content length
            match length {
                BodyLength::Sized(len) => helpers::write_content_length(len, buffer),
                BodyLength::Sized64(len) => {
                    buffer.extend_from_slice(b"content-length: ");
                    write!(buffer.writer(), "{}", len)?;
                    buffer.extend_from_slice(b"\r\n");
                }
                BodyLength::Chunked => {
                    buffer.extend_from_slice(b"transfer-encoding: chunked\r\n")
                }
                BodyLength::Empty => buffer.extend_from_slice(b"content-length: 0\r\n"),
                BodyLength::None | BodyLength::Stream => (),
            }

            let mut has_date = false;

            for (key, value) in &msg.headers {
                match *key {
                    TRANSFER_ENCODING | CONNECTION | CONTENT_LENGTH => continue,
                    DATE => has_date = true,
                    _ => (),
                }

                buffer.put_slice(key.as_ref());
                buffer.put_slice(b": ");
                buffer.put_slice(value.as_ref());
                buffer.put_slice(b"\r\n");
            }

            // Connection header
            if msg.upgrade() {
                self.flags.set(Flags::UPGRADE, msg.upgrade());
                buffer.extend_from_slice(b"connection: upgrade\r\n");
            } else if msg.keep_alive() {
                if self.version < Version::HTTP_11 {
                    buffer.extend_from_slice(b"connection: keep-alive\r\n");
                }
            } else if self.version >= Version::HTTP_11 {
                buffer.extend_from_slice(b"connection: close\r\n");
            }

            // Date header
            if !has_date {
                self.config.set_date(buffer);
            } else {
                buffer.extend_from_slice(b"\r\n");
            }
        }

        Ok(())
    }
}

impl Decoder for ClientCodec {
    type Item = ClientResponse;
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        debug_assert!(!self.inner.payload.is_some(), "Payload decoder is set");

        if let Some((req, payload)) = self.inner.decoder.decode(src)? {
            // self.inner
            //     .flags
            //     .set(Flags::HEAD, req.head.method == Method::HEAD);
            // self.inner.version = req.head.version;
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
    type Item = Message<(RequestHead, BodyLength)>;
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            Message::Item((msg, btype)) => {
                self.inner.encode_request(msg, btype, dst)?;
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

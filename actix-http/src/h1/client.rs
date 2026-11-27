use std::{fmt, io};

use bitflags::bitflags;
use bytes::{Bytes, BytesMut};
use http::{Method, Version};
use tokio_util::codec::{Decoder, Encoder};

use super::{
    decoder::{self, PayloadDecoder, PayloadItem, PayloadType},
    encoder, reserve_readbuf, Message, MessageType,
};
use crate::{
    body::BodySize,
    error::{ParseError, PayloadError},
    ConnectionType, RequestHeadType, ResponseHead, ServiceConfig,
};

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct Flags: u8 {
        const HEAD               = 0b0000_0001;
        const KEEP_ALIVE_ENABLED = 0b0000_1000;
        const STREAM             = 0b0001_0000;
    }
}

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
    conn_type: ConnectionType,

    // encoder part
    flags: Flags,
    encoder: encoder::MessageEncoder<RequestHeadType>,
}

impl Default for ClientCodec {
    fn default() -> Self {
        ClientCodec::new(ServiceConfig::default())
    }
}

impl fmt::Debug for ClientCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("h1::ClientCodec")
            .field("flags", &self.inner.flags)
            .finish_non_exhaustive()
    }
}

impl ClientCodec {
    /// Create HTTP/1 codec.
    ///
    /// `keepalive_enabled` how response `connection` header get generated.
    pub fn new(config: ServiceConfig) -> Self {
        let flags = if config.keep_alive().enabled() {
            Flags::KEEP_ALIVE_ENABLED
        } else {
            Flags::empty()
        };

        ClientCodec {
            inner: ClientCodecInner {
                config,
                decoder: decoder::MessageDecoder::default(),
                payload: None,
                version: Version::HTTP_11,
                conn_type: ConnectionType::Close,

                flags,
                encoder: encoder::MessageEncoder::default(),
            },
        }
    }

    /// Check if request is upgrade
    pub fn upgrade(&self) -> bool {
        self.inner.conn_type == ConnectionType::Upgrade
    }

    /// Check if last response is keep-alive
    pub fn keep_alive(&self) -> bool {
        self.inner.conn_type == ConnectionType::KeepAlive
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
    pub fn keep_alive(&self) -> bool {
        self.inner.conn_type == ConnectionType::KeepAlive
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
        debug_assert!(
            self.inner.payload.is_none(),
            "Payload decoder should not be set"
        );

        if let Some((req, payload)) = self.inner.decoder.decode(src)? {
            if let Some(conn_type) = req.conn_type() {
                // do not use peer's keep-alive
                self.inner.conn_type = if conn_type == ConnectionType::KeepAlive {
                    self.inner.conn_type
                } else {
                    conn_type
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

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.inner.payload.is_none() {
            return Ok(None);
        }

        if let Some(frame) = self.decode(src)? {
            return Ok(Some(frame));
        }

        // The read buffer is empty. A length or chunked body that has not
        // finished is a truncated response, not a clean end of stream.
        // RFC 9112 §6.3 item 6.
        if self
            .inner
            .payload
            .as_ref()
            .is_some_and(PayloadDecoder::expects_more)
        {
            Err(PayloadError::Incomplete(None))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;

    fn payload_after(response: &[u8]) -> (ClientPayloadCodec, BytesMut) {
        let mut codec = ClientCodec::default();
        let mut buf = BytesMut::from(response);
        codec.decode(&mut buf).unwrap().expect("response head");
        (codec.into_payload_codec(), buf)
    }

    #[tokio::test(flavor = "local")]
    async fn content_length_eof_is_incomplete() {
        let (mut codec, mut buf) = payload_after(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial",
        );
        let chunk = codec.decode(&mut buf).unwrap().unwrap().unwrap();
        assert_eq!(&chunk[..], b"partial");
        assert!(buf.is_empty());

        let err = codec.decode_eof(&mut buf).unwrap_err();
        assert!(matches!(err, PayloadError::Incomplete(None)));
    }

    #[tokio::test(flavor = "local")]
    async fn content_length_exact_eof_is_complete() {
        let (mut codec, mut buf) =
            payload_after(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata");
        let chunk = codec.decode(&mut buf).unwrap().unwrap().unwrap();
        assert_eq!(&chunk[..], b"data");

        assert!(codec.decode_eof(&mut buf).unwrap().unwrap().is_none());
    }

    #[tokio::test(flavor = "local")]
    async fn http10_close_completes_the_body() {
        let (mut codec, mut buf) = payload_after(b"HTTP/1.0 200 OK\r\n\r\nwelcome!");
        let chunk = codec.decode(&mut buf).unwrap().unwrap().unwrap();
        assert_eq!(&chunk[..], b"welcome!");

        assert!(codec.decode_eof(&mut buf).unwrap().is_none());
    }

    #[tokio::test(flavor = "local")]
    async fn chunked_eof_before_terminator_is_incomplete() {
        let (mut codec, mut buf) =
            payload_after(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello");
        let chunk = codec.decode(&mut buf).unwrap().unwrap().unwrap();
        assert_eq!(&chunk[..], b"hello");

        let err = codec.decode_eof(&mut buf).unwrap_err();
        assert!(matches!(err, PayloadError::Incomplete(None)));
    }
}

impl Encoder<Message<(RequestHeadType, BodySize)>> for ClientCodec {
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Message<(RequestHeadType, BodySize)>,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            Message::Item((mut head, length)) => {
                let inner = &mut self.inner;
                inner.version = head.as_ref().version;
                inner
                    .flags
                    .set(Flags::HEAD, head.as_ref().method == Method::HEAD);

                // connection status
                inner.conn_type = match head.as_ref().connection_type() {
                    ConnectionType::KeepAlive => {
                        if inner.flags.contains(Flags::KEEP_ALIVE_ENABLED) {
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
                    &mut head,
                    false,
                    false,
                    inner.version,
                    length,
                    inner.conn_type,
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

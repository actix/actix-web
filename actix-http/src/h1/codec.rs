use std::{fmt, io};

use bitflags::bitflags;
use bytes::BytesMut;
use http::{Method, Version};
use tokio_util::codec::{Decoder, Encoder};

use super::{
    decoder::{self, PayloadDecoder, PayloadItem, PayloadType},
    encoder, Message, MessageType,
};
use crate::{body::BodySize, error::ParseError, ConnectionType, Request, Response, ServiceConfig};

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct Flags: u8 {
        const HEAD               = 0b0000_0001;
        const KEEP_ALIVE_ENABLED = 0b0000_0010;
        const STREAM             = 0b0000_0100;
    }
}

/// HTTP/1 Codec
pub struct Codec {
    config: ServiceConfig,
    decoder: decoder::MessageDecoder<Request>,
    payload: Option<PayloadDecoder>,
    version: Version,
    conn_type: ConnectionType,

    // encoder part
    flags: Flags,
    encoder: encoder::MessageEncoder<Response<()>>,
}

impl Default for Codec {
    fn default() -> Self {
        Codec::new(ServiceConfig::default())
    }
}

impl fmt::Debug for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("h1::Codec")
            .field("flags", &self.flags)
            .finish_non_exhaustive()
    }
}

impl Codec {
    /// Create HTTP/1 codec.
    ///
    /// `keepalive_enabled` how response `connection` header get generated.
    pub fn new(config: ServiceConfig) -> Self {
        let flags = if config.keep_alive().enabled() {
            Flags::KEEP_ALIVE_ENABLED
        } else {
            Flags::empty()
        };

        Codec {
            config,
            flags,
            decoder: decoder::MessageDecoder::default(),
            payload: None,
            version: Version::HTTP_11,
            conn_type: ConnectionType::Close,
            encoder: encoder::MessageEncoder::default(),
        }
    }

    /// Check if request is upgrade.
    #[inline]
    pub fn upgrade(&self) -> bool {
        self.conn_type == ConnectionType::Upgrade
    }

    /// Check if last response is keep-alive.
    #[inline]
    pub fn keep_alive(&self) -> bool {
        self.conn_type == ConnectionType::KeepAlive
    }

    /// Check if keep-alive enabled on server level.
    #[inline]
    pub fn keep_alive_enabled(&self) -> bool {
        self.flags.contains(Flags::KEEP_ALIVE_ENABLED)
    }

    /// Check last request's message type.
    #[inline]
    pub fn message_type(&self) -> MessageType {
        if self.flags.contains(Flags::STREAM) {
            MessageType::Stream
        } else if self.payload.is_none() {
            MessageType::None
        } else {
            MessageType::Payload
        }
    }

    #[inline]
    pub fn config(&self) -> &ServiceConfig {
        &self.config
    }
}

impl Decoder for Codec {
    type Item = Message<Request>;
    type Error = ParseError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(ref mut payload) = self.payload {
            Ok(match payload.decode(src)? {
                Some(PayloadItem::Chunk(chunk)) => Some(Message::Chunk(Some(chunk))),
                Some(PayloadItem::Eof) => {
                    self.payload.take();
                    Some(Message::Chunk(None))
                }
                None => None,
            })
        } else if let Some((req, payload)) = self.decoder.decode(src)? {
            let head = req.head();
            self.flags.set(Flags::HEAD, head.method == Method::HEAD);
            self.version = head.version;
            self.conn_type = head.connection_type();

            if self.conn_type == ConnectionType::KeepAlive
                && !self.flags.contains(Flags::KEEP_ALIVE_ENABLED)
            {
                self.conn_type = ConnectionType::Close
            }

            match payload {
                PayloadType::None => self.payload = None,
                PayloadType::Payload(pl) => self.payload = Some(pl),
                PayloadType::Stream(pl) => {
                    self.payload = Some(pl);
                    self.flags.insert(Flags::STREAM);
                }
            }
            Ok(Some(Message::Item(req)))
        } else {
            Ok(None)
        }
    }
}

impl Encoder<Message<(Response<()>, BodySize)>> for Codec {
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Message<(Response<()>, BodySize)>,
        dst: &mut BytesMut,
    ) -> Result<(), Self::Error> {
        match item {
            Message::Item((mut res, length)) => {
                // set response version
                res.head_mut().version = self.version;

                // connection status
                self.conn_type = if let Some(ct) = res.head().conn_type() {
                    if ct == ConnectionType::KeepAlive {
                        self.conn_type
                    } else {
                        ct
                    }
                } else {
                    self.conn_type
                };

                // encode message
                self.encoder.encode(
                    dst,
                    &mut res,
                    self.flags.contains(Flags::HEAD),
                    self.flags.contains(Flags::STREAM),
                    self.version,
                    length,
                    self.conn_type,
                    &self.config,
                )?;
            }

            Message::Chunk(Some(bytes)) => {
                self.encoder.encode_chunk(bytes.as_ref(), dst)?;
            }

            Message::Chunk(None) => {
                self.encoder.encode_eof(dst)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HttpMessage as _;

    #[actix_rt::test]
    async fn test_http_request_chunked_payload_and_next_message() {
        let mut codec = Codec::default();

        let mut buf = BytesMut::from(
            "GET /test HTTP/1.1\r\n\
             transfer-encoding: chunked\r\n\r\n",
        );
        let item = codec.decode(&mut buf).unwrap().unwrap();
        let req = item.message();

        assert_eq!(req.method(), Method::GET);
        assert!(req.chunked().unwrap());

        buf.extend(
            b"4\r\ndata\r\n4\r\nline\r\n0\r\n\r\n\
               POST /test2 HTTP/1.1\r\n\
               transfer-encoding: chunked\r\n\r\n"
                .iter(),
        );

        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"data");

        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(msg.chunk().as_ref(), b"line");

        let msg = codec.decode(&mut buf).unwrap().unwrap();
        assert!(msg.eof());

        // decode next message
        let item = codec.decode(&mut buf).unwrap().unwrap();
        let req = item.message();
        assert_eq!(*req.method(), Method::POST);
        assert!(req.chunked().unwrap());
    }
}

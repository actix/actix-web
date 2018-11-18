#![allow(unused_imports, unused_variables, dead_code)]
use std::fmt;
use std::io::{self, Write};

use bytes::{BufMut, Bytes, BytesMut};
use tokio_codec::{Decoder, Encoder};

use super::decoder::{MessageDecoder, PayloadDecoder, PayloadItem, PayloadType};
use super::encoder::ResponseEncoder;
use super::{Message, MessageType};
use body::BodyLength;
use config::ServiceConfig;
use error::ParseError;
use helpers;
use http::header::{HeaderValue, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use http::{Method, Version};
use message::ResponseHead;
use request::Request;
use response::Response;

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
pub struct Codec {
    config: ServiceConfig,
    decoder: MessageDecoder<Request>,
    payload: Option<PayloadDecoder>,
    version: Version,

    // encoder part
    flags: Flags,
    headers_size: u32,
    te: ResponseEncoder,
}

impl Default for Codec {
    fn default() -> Self {
        Codec::new(ServiceConfig::default())
    }
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
        let flags = if config.keep_alive_enabled() {
            Flags::KEEPALIVE_ENABLED
        } else {
            Flags::empty()
        };
        Codec {
            config,
            decoder: MessageDecoder::default(),
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
        if self.flags.contains(Flags::STREAM) {
            MessageType::Stream
        } else if self.payload.is_none() {
            MessageType::None
        } else {
            MessageType::Payload
        }
    }

    /// prepare transfer encoding
    pub fn prepare_te(&mut self, head: &mut ResponseHead, length: &mut BodyLength) {
        self.te
            .update(head, self.flags.contains(Flags::HEAD), self.version, length);
    }

    fn encode_response(
        &mut self,
        mut msg: Response<()>,
        buffer: &mut BytesMut,
    ) -> io::Result<()> {
        let ka = self.flags.contains(Flags::KEEPALIVE_ENABLED) && msg
            .keep_alive()
            .unwrap_or_else(|| self.flags.contains(Flags::KEEPALIVE));

        // Connection upgrade
        if msg.upgrade() {
            self.flags.insert(Flags::UPGRADE);
            self.flags.remove(Flags::KEEPALIVE);
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("upgrade"));
        }
        // keep-alive
        else if ka {
            self.flags.insert(Flags::KEEPALIVE);
            if self.version < Version::HTTP_11 {
                msg.headers_mut()
                    .insert(CONNECTION, HeaderValue::from_static("keep-alive"));
            }
        } else if self.version >= Version::HTTP_11 {
            self.flags.remove(Flags::KEEPALIVE);
            msg.headers_mut()
                .insert(CONNECTION, HeaderValue::from_static("close"));
        }

        // render message
        {
            let reason = msg.reason().as_bytes();
            buffer
                .reserve(256 + msg.headers().len() * AVERAGE_HEADER_SIZE + reason.len());

            // status line
            helpers::write_status_line(self.version, msg.status().as_u16(), buffer);
            buffer.extend_from_slice(reason);

            // content length
            let mut len_is_set = true;
            match self.te.length {
                BodyLength::Chunked => {
                    buffer.extend_from_slice(b"\r\ntransfer-encoding: chunked\r\n")
                }
                BodyLength::Empty => {
                    len_is_set = false;
                    buffer.extend_from_slice(b"\r\n")
                }
                BodyLength::Sized(len) => helpers::write_content_length(len, buffer),
                BodyLength::Sized64(len) => {
                    buffer.extend_from_slice(b"\r\ncontent-length: ");
                    write!(buffer.writer(), "{}", len)?;
                    buffer.extend_from_slice(b"\r\n");
                }
                BodyLength::None | BodyLength::Stream => {
                    buffer.extend_from_slice(b"\r\n")
                }
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
                        BodyLength::None => (),
                        BodyLength::Empty => {
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
                Some(PayloadItem::Eof) => {
                    self.payload.take();
                    Some(Message::Chunk(None))
                }
                None => None,
            })
        } else if let Some((req, payload)) = self.decoder.decode(src)? {
            self.flags
                .set(Flags::HEAD, req.inner.head.method == Method::HEAD);
            self.version = req.inner.head.version;
            if self.flags.contains(Flags::KEEPALIVE_ENABLED) {
                self.flags.set(Flags::KEEPALIVE, req.keep_alive());
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

impl Encoder for Codec {
    type Item = Message<Response<()>>;
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut BytesMut,
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

#[cfg(test)]
mod tests {
    use std::{cmp, io};

    use bytes::{Buf, Bytes, BytesMut};
    use http::{Method, Version};
    use tokio_io::{AsyncRead, AsyncWrite};

    use super::*;
    use error::ParseError;
    use h1::Message;
    use httpmessage::HttpMessage;
    use request::Request;

    #[test]
    fn test_http_request_chunked_payload_and_next_message() {
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

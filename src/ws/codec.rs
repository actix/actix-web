use bytes::BytesMut;
use tokio_codec::{Decoder, Encoder};

use super::frame::Frame;
use super::proto::{CloseReason, OpCode};
use super::ProtocolError;
use body::Binary;

/// `WebSocket` Message
#[derive(Debug, PartialEq)]
pub enum Message {
    /// Text message
    Text(String),
    /// Binary message
    Binary(Binary),
    /// Ping message
    Ping(String),
    /// Pong message
    Pong(String),
    /// Close message with optional reason
    Close(Option<CloseReason>),
}

/// WebSockets protocol codec
pub struct Codec {
    max_size: usize,
    server: bool,
}

impl Codec {
    /// Create new websocket frames decoder
    pub fn new() -> Codec {
        Codec {
            max_size: 65_536,
            server: true,
        }
    }

    /// Set max frame size
    ///
    /// By default max size is set to 64kb
    pub fn max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Set decoder to client mode.
    ///
    /// By default decoder works in server mode.
    pub fn client_mode(mut self) -> Self {
        self.server = false;
        self
    }
}

impl Encoder for Codec {
    type Item = Message;
    type Error = ProtocolError;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        match item {
            Message::Text(txt) => {
                Frame::write_message(dst, txt, OpCode::Text, true, !self.server)
            }
            Message::Binary(bin) => {
                Frame::write_message(dst, bin, OpCode::Binary, true, !self.server)
            }
            Message::Ping(txt) => {
                Frame::write_message(dst, txt, OpCode::Ping, true, !self.server)
            }
            Message::Pong(txt) => {
                Frame::write_message(dst, txt, OpCode::Pong, true, !self.server)
            }
            Message::Close(reason) => Frame::write_close(dst, reason, !self.server),
        }
        Ok(())
    }
}

impl Decoder for Codec {
    type Item = Message;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match Frame::parse(src, self.server, self.max_size) {
            Ok(Some((finished, opcode, payload))) => {
                // continuation is not supported
                if !finished {
                    return Err(ProtocolError::NoContinuation);
                }

                match opcode {
                    OpCode::Continue => Err(ProtocolError::NoContinuation),
                    OpCode::Bad => Err(ProtocolError::BadOpCode),
                    OpCode::Close => {
                        let close_reason = Frame::parse_close_payload(&payload);
                        Ok(Some(Message::Close(close_reason)))
                    }
                    OpCode::Ping => Ok(Some(Message::Ping(
                        String::from_utf8_lossy(payload.as_ref()).into(),
                    ))),
                    OpCode::Pong => Ok(Some(Message::Pong(
                        String::from_utf8_lossy(payload.as_ref()).into(),
                    ))),
                    OpCode::Binary => Ok(Some(Message::Binary(payload))),
                    OpCode::Text => {
                        let tmp = Vec::from(payload.as_ref());
                        match String::from_utf8(tmp) {
                            Ok(s) => Ok(Some(Message::Text(s))),
                            Err(_) => Err(ProtocolError::BadEncoding),
                        }
                    }
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

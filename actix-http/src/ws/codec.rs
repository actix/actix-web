use actix_codec::{Decoder, Encoder};
use bytes::{Bytes, BytesMut};

use super::frame::Parser;
use super::proto::{CloseReason, OpCode};
use super::ProtocolError;

/// `WebSocket` Message
#[derive(Debug, PartialEq)]
pub enum Message {
    /// Text message
    Text(String),
    /// Binary message
    Binary(Bytes),
    /// Ping message
    Ping(String),
    /// Pong message
    Pong(String),
    /// Close message with optional reason
    Close(Option<CloseReason>),
    /// No-op. Useful for actix-net services
    Nop,
}

/// `WebSocket` frame
#[derive(Debug, PartialEq)]
pub enum Frame {
    /// Text frame, codec does not verify utf8 encoding
    Text(Option<BytesMut>),
    /// Binary frame
    Binary(Option<BytesMut>),
    /// Ping message
    Ping(String),
    /// Pong message
    Pong(String),
    /// Close message with optional reason
    Close(Option<CloseReason>),
}

#[derive(Debug, Copy, Clone)]
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
                Parser::write_message(dst, txt, OpCode::Text, true, !self.server)
            }
            Message::Binary(bin) => {
                Parser::write_message(dst, bin, OpCode::Binary, true, !self.server)
            }
            Message::Ping(txt) => {
                Parser::write_message(dst, txt, OpCode::Ping, true, !self.server)
            }
            Message::Pong(txt) => {
                Parser::write_message(dst, txt, OpCode::Pong, true, !self.server)
            }
            Message::Close(reason) => Parser::write_close(dst, reason, !self.server),
            Message::Nop => (),
        }
        Ok(())
    }
}

impl Decoder for Codec {
    type Item = Frame;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match Parser::parse(src, self.server, self.max_size) {
            Ok(Some((finished, opcode, payload))) => {
                // continuation is not supported
                if !finished {
                    return Err(ProtocolError::NoContinuation);
                }

                match opcode {
                    OpCode::Continue => Err(ProtocolError::NoContinuation),
                    OpCode::Bad => Err(ProtocolError::BadOpCode),
                    OpCode::Close => {
                        if let Some(ref pl) = payload {
                            let close_reason = Parser::parse_close_payload(pl);
                            Ok(Some(Frame::Close(close_reason)))
                        } else {
                            Ok(Some(Frame::Close(None)))
                        }
                    }
                    OpCode::Ping => {
                        if let Some(ref pl) = payload {
                            Ok(Some(Frame::Ping(String::from_utf8_lossy(pl).into())))
                        } else {
                            Ok(Some(Frame::Ping(String::new())))
                        }
                    }
                    OpCode::Pong => {
                        if let Some(ref pl) = payload {
                            Ok(Some(Frame::Pong(String::from_utf8_lossy(pl).into())))
                        } else {
                            Ok(Some(Frame::Pong(String::new())))
                        }
                    }
                    OpCode::Binary => Ok(Some(Frame::Binary(payload))),
                    OpCode::Text => {
                        Ok(Some(Frame::Text(payload)))
                        //let tmp = Vec::from(payload.as_ref());
                        //match String::from_utf8(tmp) {
                        //    Ok(s) => Ok(Some(Message::Text(s))),
                        //    Err(_) => Err(ProtocolError::BadEncoding),
                        //}
                    }
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

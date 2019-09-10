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
    Ping(Bytes),
    /// Pong message
    Pong(Bytes),
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
    Ping(Bytes),
    /// Pong message
    Pong(Bytes),
    /// Close message with optional reason
    Close(Option<CloseReason>),
    /// Active continuation
    Continue,
}

#[derive(Debug, Clone)]
/// WebSockets protocol codec
pub struct Codec {
    max_size: usize,
    server: bool,
    cont_code: Option<OpCode>,
    buf: Vec<BytesMut>,
}

impl Codec {
    /// Create new websocket frames decoder
    pub fn new() -> Codec {
        Codec {
            max_size: 65_536,
            server: true,
            buf: vec![],
            cont_code: None,
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

    fn combine_payload(&mut self, payload: Option<BytesMut>) -> Option<BytesMut> {
        let mut size: usize = if let Some(ref pl) = payload {
            pl.len()
        } else {
            0
        };
        size += self.buf.iter().map(|pl| pl.len()).sum::<usize>();
        if size > 0 {
            let mut res = BytesMut::with_capacity(size);
            for pl in self.buf.drain(..) {
                res.extend_from_slice(&pl)
            }
            if let Some(pl) = payload {
                res.extend_from_slice(&pl)
            }
            Some(res)
        } else {
            None
        }
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
            Ok(Some((finished, rsv, mut opcode, mut payload))) => {
                // Since this is the default codec we have no extension
                // and should fail if rsv is set.
                // In an async context this will cause a NON-STRICT
                // autoban complience since we might terminate
                // before receiving the prior message.
                if rsv != 0 {
                    return Err(ProtocolError::RSVSet);
                }

                if !finished {
                    if (opcode == OpCode::Text || opcode == OpCode::Binary)
                        && self.cont_code.is_none()
                    {
                        // We are starting a new continuation
                        self.cont_code = Some(opcode);
                        if let Some(pl) = payload {
                            self.buf.push(pl);
                        }
                        return Ok(Some(Frame::Continue));
                    } else if opcode == OpCode::Continue && self.cont_code.is_some() {
                        // We continue a continuation
                        if let Some(pl) = payload {
                            self.buf.push(pl);
                        };
                        return Ok(Some(Frame::Continue));
                    } else {
                        return Err(ProtocolError::NoContinuation);
                    }
                } else if opcode == OpCode::Continue {
                    // We finish a continuation
                    if let Some(orig_opcode) = self.cont_code {
                        // reset saved opcode
                        self.cont_code = None;
                        // put cached code into current opciode
                        opcode = orig_opcode;
                        // Collect the payload
                        payload = self.combine_payload(payload)
                    } else {
                        // We have a continuation finish op code but nothing to continue,
                        // this is an error
                        return Err(ProtocolError::NoContinuation);
                    }
                } else if self.cont_code.is_some()
                    && (opcode == OpCode::Binary || opcode == OpCode::Text)
                {
                    // We are finished but this isn't a continuation and
                    // we still have a started continuation
                    return Err(ProtocolError::NoContinuation);
                }

                match opcode {
                    OpCode::Continue => unreachable!(),
                    OpCode::Bad => Err(ProtocolError::BadOpCode),
                    OpCode::Close => {
                        if let Some(ref pl) = payload {
                            let close_reason = Parser::parse_close_payload(pl)?;
                            Ok(Some(Frame::Close(close_reason)))
                        } else {
                            Ok(Some(Frame::Close(None)))
                        }
                    }
                    OpCode::Ping => {
                        if let Some(pl) = payload {
                            Ok(Some(Frame::Ping(pl.into())))
                        } else {
                            Ok(Some(Frame::Ping(Bytes::new())))
                        }
                    }
                    OpCode::Pong => {
                        if let Some(pl) = payload {
                            Ok(Some(Frame::Pong(pl.into())))
                        } else {
                            Ok(Some(Frame::Pong(Bytes::new())))
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

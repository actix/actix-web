use bitflags::bitflags;
use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use tokio_util::codec::{Decoder, Encoder};
use tracing::error;

use super::{
    frame::Parser,
    proto::{CloseReason, OpCode},
    ProtocolError,
};

/// A WebSocket message.
#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    /// Text message.
    Text(ByteString),

    /// Binary message.
    Binary(Bytes),

    /// Continuation.
    Continuation(Item),

    /// Ping message.
    Ping(Bytes),

    /// Pong message.
    Pong(Bytes),

    /// Close message with optional reason.
    Close(Option<CloseReason>),

    /// No-op. Useful for low-level services.
    Nop,
}

/// A WebSocket frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// Text frame. Note that the codec does not validate UTF-8 encoding.
    Text(Bytes),

    /// Binary frame.
    Binary(Bytes),

    /// Continuation.
    Continuation(Item),

    /// Ping message.
    Ping(Bytes),

    /// Pong message.
    Pong(Bytes),

    /// Close message with optional reason.
    Close(Option<CloseReason>),
}

/// A WebSocket continuation item.
#[derive(Debug, PartialEq, Eq)]
pub enum Item {
    FirstText(Bytes),
    FirstBinary(Bytes),
    Continue(Bytes),
    Last(Bytes),
}

/// WebSocket protocol codec.
#[derive(Debug, Clone)]
pub struct Codec {
    flags: Flags,
    max_size: usize,
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct Flags: u8 {
        const SERVER         = 0b0000_0001;
        const CONTINUATION   = 0b0000_0010;
        const W_CONTINUATION = 0b0000_0100;
    }
}

impl Codec {
    /// Create new WebSocket frames decoder.
    pub const fn new() -> Codec {
        Codec {
            max_size: 65_536,
            flags: Flags::SERVER,
        }
    }

    /// Set max frame size.
    ///
    /// By default max size is set to 64KiB.
    #[must_use = "This returns the a new Codec, without modifying the original."]
    pub fn max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Set decoder to client mode.
    ///
    /// By default decoder works in server mode.
    #[must_use = "This returns the a new Codec, without modifying the original."]
    pub fn client_mode(mut self) -> Self {
        self.flags.remove(Flags::SERVER);
        self
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder<Message> for Codec {
    type Error = ProtocolError;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        match item {
            Message::Text(txt) => Parser::write_message(
                dst,
                txt,
                OpCode::Text,
                true,
                !self.flags.contains(Flags::SERVER),
            ),
            Message::Binary(bin) => Parser::write_message(
                dst,
                bin,
                OpCode::Binary,
                true,
                !self.flags.contains(Flags::SERVER),
            ),
            Message::Ping(txt) => Parser::write_message(
                dst,
                txt,
                OpCode::Ping,
                true,
                !self.flags.contains(Flags::SERVER),
            ),
            Message::Pong(txt) => Parser::write_message(
                dst,
                txt,
                OpCode::Pong,
                true,
                !self.flags.contains(Flags::SERVER),
            ),
            Message::Close(reason) => {
                Parser::write_close(dst, reason, !self.flags.contains(Flags::SERVER))
            }
            Message::Continuation(cont) => match cont {
                Item::FirstText(data) => {
                    if self.flags.contains(Flags::W_CONTINUATION) {
                        return Err(ProtocolError::ContinuationStarted);
                    } else {
                        self.flags.insert(Flags::W_CONTINUATION);
                        Parser::write_message(
                            dst,
                            &data[..],
                            OpCode::Text,
                            false,
                            !self.flags.contains(Flags::SERVER),
                        )
                    }
                }
                Item::FirstBinary(data) => {
                    if self.flags.contains(Flags::W_CONTINUATION) {
                        return Err(ProtocolError::ContinuationStarted);
                    } else {
                        self.flags.insert(Flags::W_CONTINUATION);
                        Parser::write_message(
                            dst,
                            &data[..],
                            OpCode::Binary,
                            false,
                            !self.flags.contains(Flags::SERVER),
                        )
                    }
                }
                Item::Continue(data) => {
                    if self.flags.contains(Flags::W_CONTINUATION) {
                        Parser::write_message(
                            dst,
                            &data[..],
                            OpCode::Continue,
                            false,
                            !self.flags.contains(Flags::SERVER),
                        )
                    } else {
                        return Err(ProtocolError::ContinuationNotStarted);
                    }
                }
                Item::Last(data) => {
                    if self.flags.contains(Flags::W_CONTINUATION) {
                        self.flags.remove(Flags::W_CONTINUATION);
                        Parser::write_message(
                            dst,
                            &data[..],
                            OpCode::Continue,
                            true,
                            !self.flags.contains(Flags::SERVER),
                        )
                    } else {
                        return Err(ProtocolError::ContinuationNotStarted);
                    }
                }
            },
            Message::Nop => {}
        }
        Ok(())
    }
}

impl Decoder for Codec {
    type Item = Frame;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match Parser::parse(src, self.flags.contains(Flags::SERVER), self.max_size) {
            Ok(Some((finished, opcode, payload))) => {
                // continuation is not supported
                if !finished {
                    return match opcode {
                        OpCode::Continue => {
                            if self.flags.contains(Flags::CONTINUATION) {
                                Ok(Some(Frame::Continuation(Item::Continue(
                                    payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationNotStarted)
                            }
                        }
                        OpCode::Binary => {
                            if !self.flags.contains(Flags::CONTINUATION) {
                                self.flags.insert(Flags::CONTINUATION);
                                Ok(Some(Frame::Continuation(Item::FirstBinary(
                                    payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationStarted)
                            }
                        }
                        OpCode::Text => {
                            if !self.flags.contains(Flags::CONTINUATION) {
                                self.flags.insert(Flags::CONTINUATION);
                                Ok(Some(Frame::Continuation(Item::FirstText(
                                    payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationStarted)
                            }
                        }
                        _ => {
                            error!("Unfinished fragment {:?}", opcode);
                            Err(ProtocolError::ContinuationFragment(opcode))
                        }
                    };
                }

                match opcode {
                    OpCode::Continue => {
                        if self.flags.contains(Flags::CONTINUATION) {
                            self.flags.remove(Flags::CONTINUATION);
                            Ok(Some(Frame::Continuation(Item::Last(
                                payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                            ))))
                        } else {
                            Err(ProtocolError::ContinuationNotStarted)
                        }
                    }
                    OpCode::Bad => Err(ProtocolError::BadOpCode),
                    OpCode::Close => {
                        if let Some(ref pl) = payload {
                            let close_reason = Parser::parse_close_payload(pl);
                            Ok(Some(Frame::Close(close_reason)))
                        } else {
                            Ok(Some(Frame::Close(None)))
                        }
                    }
                    OpCode::Ping => Ok(Some(Frame::Ping(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                    OpCode::Pong => Ok(Some(Frame::Pong(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                    OpCode::Binary => Ok(Some(Frame::Binary(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                    OpCode::Text => Ok(Some(Frame::Text(
                        payload.map(|pl| pl.freeze()).unwrap_or_else(Bytes::new),
                    ))),
                }
            }
            Ok(None) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

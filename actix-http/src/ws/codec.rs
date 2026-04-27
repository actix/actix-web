use bitflags::bitflags;
use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use tokio_util::codec;
use tracing::error;

#[cfg(feature = "compress-ws-deflate")]
use super::deflate::{
    DeflateCompressionContext, DeflateDecompressionContext, RSV_BIT_DEFLATE_FLAG,
};
use super::{
    frame::Parser,
    proto::{CloseReason, OpCode, RsvBits},
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

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct Flags: u8 {
        const SERVER         = 0b0000_0001;
        const CONTINUATION   = 0b0000_0010;
        const W_CONTINUATION = 0b0000_0100;
    }
}

/// WebSocket message encoder.
#[derive(Debug)]
pub struct Encoder {
    flags: Flags,

    #[cfg(feature = "compress-ws-deflate")]
    deflate_compress: Option<DeflateCompressionContext>,
}

impl Encoder {
    /// Create new WebSocket frames encoder.
    pub const fn new() -> Encoder {
        Encoder {
            flags: Flags::SERVER,

            #[cfg(feature = "compress-ws-deflate")]
            deflate_compress: None,
        }
    }

    /// Create new WebSocket frames encoder with `permessage-deflate` extension support.
    /// Compression context can be made from
    /// [`DeflateSessionParameters::create_context`](super::DeflateSessionParameters::create_context).
    #[cfg(feature = "compress-ws-deflate")]
    pub fn new_deflate(compress: DeflateCompressionContext) -> Encoder {
        Encoder {
            flags: Flags::SERVER,

            deflate_compress: Some(compress),
        }
    }

    /// Set encoder to client mode.
    ///
    /// By default encoder works in server mode.
    #[must_use = "This returns the a new Encoder, without modifying the original."]
    pub fn client_mode(mut self) -> Self {
        self.flags = Flags::empty();
        self
    }

    #[cfg(feature = "compress-ws-deflate")]
    fn set_client_mode_deflate(
        mut self,
        remote_no_context_takeover: bool,
        remote_max_window_bits: u8,
    ) -> Self {
        self.deflate_compress = self
            .deflate_compress
            .map(|c| c.reset_with(remote_no_context_takeover, remote_max_window_bits));
        self
    }

    #[cfg(feature = "compress-ws-deflate")]
    fn process_payload(
        &mut self,
        fin: bool,
        bytes: Bytes,
    ) -> Result<(Bytes, RsvBits), ProtocolError> {
        if let Some(compress) = &mut self.deflate_compress {
            Ok((compress.compress(fin, bytes)?, RSV_BIT_DEFLATE_FLAG))
        } else {
            Ok((bytes, RsvBits::empty()))
        }
    }

    #[cfg(not(feature = "compress-ws-deflate"))]
    fn process_payload(
        &mut self,
        _fin: bool,
        bytes: Bytes,
    ) -> Result<(Bytes, RsvBits), ProtocolError> {
        Ok((bytes, RsvBits::empty()))
    }
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl codec::Encoder<Message> for Encoder {
    type Error = ProtocolError;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        match item {
            Message::Text(txt) => {
                let (bytes, rsv_bits) = self.process_payload(true, txt.into_bytes())?;

                Parser::write_message(
                    dst,
                    bytes,
                    OpCode::Text,
                    rsv_bits,
                    true,
                    !self.flags.contains(Flags::SERVER),
                )
            }
            Message::Binary(bin) => {
                let (bin, rsv_bits) = self.process_payload(true, bin)?;

                Parser::write_message(
                    dst,
                    bin,
                    OpCode::Binary,
                    rsv_bits,
                    true,
                    !self.flags.contains(Flags::SERVER),
                )
            }
            Message::Ping(txt) => Parser::write_message(
                dst,
                txt,
                OpCode::Ping,
                RsvBits::empty(),
                true,
                !self.flags.contains(Flags::SERVER),
            ),
            Message::Pong(txt) => Parser::write_message(
                dst,
                txt,
                OpCode::Pong,
                RsvBits::empty(),
                true,
                !self.flags.contains(Flags::SERVER),
            ),
            Message::Close(reason) => Parser::write_close(
                dst,
                reason,
                RsvBits::empty(),
                !self.flags.contains(Flags::SERVER),
            ),
            Message::Continuation(cont) => match cont {
                Item::FirstText(data) => {
                    if self.flags.contains(Flags::W_CONTINUATION) {
                        return Err(ProtocolError::ContinuationStarted);
                    } else {
                        let (data, rsv_bits) = self.process_payload(false, data)?;

                        self.flags.insert(Flags::W_CONTINUATION);
                        Parser::write_message(
                            dst,
                            data,
                            OpCode::Text,
                            rsv_bits,
                            false,
                            !self.flags.contains(Flags::SERVER),
                        )
                    }
                }
                Item::FirstBinary(data) => {
                    if self.flags.contains(Flags::W_CONTINUATION) {
                        return Err(ProtocolError::ContinuationStarted);
                    } else {
                        let (data, rsv_bits) = self.process_payload(false, data)?;

                        self.flags.insert(Flags::W_CONTINUATION);
                        Parser::write_message(
                            dst,
                            data,
                            OpCode::Binary,
                            rsv_bits,
                            false,
                            !self.flags.contains(Flags::SERVER),
                        )
                    }
                }
                Item::Continue(data) => {
                    if self.flags.contains(Flags::W_CONTINUATION) {
                        let (data, rsv_bits) = self.process_payload(false, data)?;

                        Parser::write_message(
                            dst,
                            data,
                            OpCode::Continue,
                            rsv_bits,
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

                        let (data, rsv_bits) = self.process_payload(true, data)?;

                        Parser::write_message(
                            dst,
                            data,
                            OpCode::Continue,
                            rsv_bits,
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

/// WebSocket message decoder.
#[derive(Debug)]
pub struct Decoder {
    flags: Flags,
    max_size: usize,

    #[cfg(feature = "compress-ws-deflate")]
    deflate_decompress: Option<DeflateDecompressionContext>,
}

impl Decoder {
    /// Create new WebSocket frames decoder.
    pub const fn new() -> Decoder {
        Decoder {
            flags: Flags::SERVER,
            max_size: 65_536,

            #[cfg(feature = "compress-ws-deflate")]
            deflate_decompress: None,
        }
    }

    /// Create new WebSocket frames decoder with `permessage-deflate` extension support.
    /// Decompression context can be made from
    /// [`DeflateSessionParameters::create_context`](super::DeflateSessionParameters::create_context).
    #[cfg(feature = "compress-ws-deflate")]
    pub fn new_deflate(decompress: DeflateDecompressionContext) -> Decoder {
        Decoder {
            flags: Flags::SERVER,
            max_size: 65_536,

            deflate_decompress: Some(decompress),
        }
    }

    /// Set max frame size.
    ///
    /// By default max size is set to 64KiB.
    #[must_use = "This returns the a new Decoder, without modifying the original."]
    pub fn max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Set decoder to client mode.
    ///
    /// By default decoder works in server mode.
    #[must_use = "This returns the a new Decoder, without modifying the original."]
    pub fn client_mode(mut self) -> Self {
        self.flags = Flags::empty();
        self
    }

    #[cfg(feature = "compress-ws-deflate")]
    fn set_client_mode_deflate(
        mut self,
        local_no_context_takeover: bool,
        local_max_window_bits: u8,
    ) -> Self {
        if let Some(decompress) = &mut self.deflate_decompress {
            decompress.reset_with(local_no_context_takeover, local_max_window_bits);
        }

        self
    }

    #[cfg(feature = "compress-ws-deflate")]
    fn process_payload(
        &mut self,
        fin: bool,
        opcode: OpCode,
        rsv_bits: RsvBits,
        bytes: Option<Bytes>,
    ) -> Result<Option<Bytes>, ProtocolError> {
        if let Some(bytes) = bytes {
            if let Some(decompress) = &mut self.deflate_decompress {
                Ok(Some(decompress.decompress(fin, opcode, rsv_bits, bytes)?))
            } else {
                Ok(Some(bytes))
            }
        } else {
            Ok(None)
        }
    }

    #[cfg(not(feature = "compress-ws-deflate"))]
    fn process_payload(
        &mut self,
        _fin: bool,
        _opcode: OpCode,
        _rsv_bits: RsvBits,
        bytes: Option<Bytes>,
    ) -> Result<Option<Bytes>, ProtocolError> {
        Ok(bytes)
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl codec::Decoder for Decoder {
    type Item = Frame;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match Parser::parse(src, self.flags.contains(Flags::SERVER), self.max_size) {
            Ok(Some((finished, opcode, rsv_bits, payload))) => {
                let payload = self.process_payload(
                    finished,
                    opcode,
                    rsv_bits,
                    payload.map(BytesMut::freeze),
                )?;

                // continuation is not supported
                if !finished {
                    return match opcode {
                        OpCode::Continue => {
                            if self.flags.contains(Flags::CONTINUATION) {
                                Ok(Some(Frame::Continuation(Item::Continue(
                                    payload.unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationNotStarted)
                            }
                        }
                        OpCode::Binary => {
                            if !self.flags.contains(Flags::CONTINUATION) {
                                self.flags.insert(Flags::CONTINUATION);
                                Ok(Some(Frame::Continuation(Item::FirstBinary(
                                    payload.unwrap_or_else(Bytes::new),
                                ))))
                            } else {
                                Err(ProtocolError::ContinuationStarted)
                            }
                        }
                        OpCode::Text => {
                            if !self.flags.contains(Flags::CONTINUATION) {
                                self.flags.insert(Flags::CONTINUATION);
                                Ok(Some(Frame::Continuation(Item::FirstText(
                                    payload.unwrap_or_else(Bytes::new),
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
                                payload.unwrap_or_else(Bytes::new),
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
                    OpCode::Ping => Ok(Some(Frame::Ping(payload.unwrap_or_else(Bytes::new)))),
                    OpCode::Pong => Ok(Some(Frame::Pong(payload.unwrap_or_else(Bytes::new)))),
                    OpCode::Binary => Ok(Some(Frame::Binary(payload.unwrap_or_else(Bytes::new)))),
                    OpCode::Text => Ok(Some(Frame::Text(payload.unwrap_or_else(Bytes::new)))),
                }
            }
            Ok(None) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

/// WebSocket protocol codec.
/// This is essentially a combination of [`Encoder`] and [`Decoder`] and
/// actual conversion behaviors are defined in both structs respectively.
///
/// # Note
/// Cloning [`Codec`] creates a new codec with existing configurations
/// and will not preserve the context information.
#[derive(Debug, Default)]
pub struct Codec {
    encoder: Encoder,
    decoder: Decoder,
}

impl Clone for Codec {
    fn clone(&self) -> Self {
        Self {
            encoder: Encoder {
                flags: self.encoder.flags & Flags::SERVER,
                #[cfg(feature = "compress-ws-deflate")]
                deflate_compress: self.encoder.deflate_compress.as_ref().map(|c| {
                    DeflateCompressionContext::new(
                        Some(c.compression_level),
                        c.remote_no_context_takeover,
                        c.remote_max_window_bits,
                    )
                }),
            },
            decoder: Decoder {
                flags: self.decoder.flags & Flags::SERVER,
                max_size: self.decoder.max_size,
                #[cfg(feature = "compress-ws-deflate")]
                deflate_decompress: self.decoder.deflate_decompress.as_ref().map(|d| {
                    DeflateDecompressionContext::new(
                        d.local_no_context_takeover,
                        d.local_max_window_bits,
                    )
                }),
            },
        }
    }
}

impl Codec {
    /// Create new WebSocket frames codec.
    pub fn new() -> Codec {
        Codec {
            encoder: Encoder::new(),
            decoder: Decoder::new(),
        }
    }

    /// Create new WebSocket frames codec with DEFLATE compression.
    /// Both compression and decompression contexts can be made from
    /// [`DeflateSessionParameters::create_context`](super::DeflateSessionParameters::create_context).
    #[cfg(feature = "compress-ws-deflate")]
    pub fn new_deflate(
        compress: DeflateCompressionContext,
        decompress: DeflateDecompressionContext,
    ) -> Codec {
        Codec {
            encoder: Encoder::new_deflate(compress),
            decoder: Decoder::new_deflate(decompress),
        }
    }

    /// Set max frame size.
    ///
    /// By default max size is set to 64KiB.
    #[must_use = "This returns the a new Codec, without modifying the original."]
    pub fn max_size(self, size: usize) -> Self {
        let Self { encoder, decoder } = self;

        Codec {
            encoder,
            decoder: decoder.max_size(size),
        }
    }

    /// Set codec to client mode.
    ///
    /// By default codec works in server mode.
    #[must_use = "This returns the a new Codec, without modifying the original."]
    pub fn client_mode(self) -> Self {
        let Self {
            mut encoder,
            mut decoder,
        } = self;

        encoder = encoder.client_mode();
        decoder = decoder.client_mode();
        #[cfg(feature = "compress-ws-deflate")]
        {
            if let Some(decoder) = &decoder.deflate_decompress {
                encoder = encoder.set_client_mode_deflate(
                    decoder.local_no_context_takeover,
                    decoder.local_max_window_bits,
                );
            }
            if let Some(encoder) = &encoder.deflate_compress {
                decoder = decoder.set_client_mode_deflate(
                    encoder.remote_no_context_takeover,
                    encoder.remote_max_window_bits,
                );
            }
        }

        Self { encoder, decoder }
    }
}

impl codec::Decoder for Codec {
    type Item = Frame;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode(src)
    }
}

impl codec::Encoder<Message> for Codec {
    type Error = ProtocolError;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        self.encoder.encode(item, dst)
    }
}

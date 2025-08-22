use std::cmp::min;

use bytes::{Buf, BufMut, BytesMut};
use tracing::debug;

use super::{
    mask::apply_mask,
    proto::{CloseCode, CloseReason, OpCode, RsvBits},
    ProtocolError,
};

/// A struct representing a WebSocket frame.
#[derive(Debug)]
pub struct Parser;

impl Parser {
    fn parse_metadata(
        src: &[u8],
        server: bool,
    ) -> Result<Option<(usize, bool, OpCode, RsvBits, usize, Option<[u8; 4]>)>, ProtocolError> {
        let chunk_len = src.len();

        let mut idx = 2;
        if chunk_len < 2 {
            return Ok(None);
        }

        let first = src[0];
        let second = src[1];
        let finished = first & 0x80 != 0;

        // check masking
        let masked = second & 0x80 != 0;
        if !masked && server {
            return Err(ProtocolError::UnmaskedFrame);
        } else if masked && !server {
            return Err(ProtocolError::MaskedFrame);
        }

        // RSV bits
        let rsv_bits = RsvBits::from_bits((first & 0x70) >> 4).unwrap_or(RsvBits::empty());

        // Op code
        let opcode = OpCode::from(first & 0x0F);

        if let OpCode::Bad = opcode {
            return Err(ProtocolError::InvalidOpcode(first & 0x0F));
        }

        let len = second & 0x7F;
        let length = if len == 126 {
            if chunk_len < 4 {
                return Ok(None);
            }
            let len = usize::from(u16::from_be_bytes(
                TryFrom::try_from(&src[idx..idx + 2]).unwrap(),
            ));
            idx += 2;
            len
        } else if len == 127 {
            if chunk_len < 10 {
                return Ok(None);
            }
            let len = u64::from_be_bytes(TryFrom::try_from(&src[idx..idx + 8]).unwrap());
            idx += 8;
            len as usize
        } else {
            len as usize
        };

        let mask = if server {
            if chunk_len < idx + 4 {
                return Ok(None);
            }

            let mask = TryFrom::try_from(&src[idx..idx + 4]).unwrap();

            idx += 4;

            Some(mask)
        } else {
            None
        };

        Ok(Some((idx, finished, opcode, rsv_bits, length, mask)))
    }

    /// Parse the input stream into a frame.
    pub fn parse(
        src: &mut BytesMut,
        server: bool,
        max_size: usize,
    ) -> Result<Option<(bool, OpCode, RsvBits, Option<BytesMut>)>, ProtocolError> {
        // try to parse ws frame metadata
        let (idx, finished, opcode, rsv_bits, length, mask) =
            match Parser::parse_metadata(src, server)? {
                None => return Ok(None),
                Some(res) => res,
            };

        // not enough data
        if src.len() < idx + length {
            let min_length = min(length, max_size);
            if src.capacity() < idx + min_length {
                src.reserve(idx + min_length - src.capacity());
            }
            return Ok(None);
        }

        // remove prefix
        src.advance(idx);

        // check for max allowed size
        if length > max_size {
            // drop the payload
            src.advance(length);
            return Err(ProtocolError::Overflow);
        }

        // no need for body
        if length == 0 {
            return Ok(Some((finished, opcode, rsv_bits, None)));
        }

        let mut data = src.split_to(length);

        // control frames must have length <= 125
        match opcode {
            OpCode::Ping | OpCode::Pong if length > 125 => {
                return Err(ProtocolError::InvalidLength(length));
            }
            OpCode::Close if length > 125 => {
                debug!("Received close frame with payload length exceeding 125. Morphing to protocol close frame.");
                return Ok(Some((true, OpCode::Close, rsv_bits, None)));
            }
            _ => {}
        }

        // unmask
        if let Some(mask) = mask {
            apply_mask(&mut data, mask);
        }

        Ok(Some((finished, opcode, rsv_bits, Some(data))))
    }

    /// Parse the payload of a close frame.
    pub fn parse_close_payload(payload: &[u8]) -> Option<CloseReason> {
        if payload.len() >= 2 {
            let raw_code = u16::from_be_bytes(TryFrom::try_from(&payload[..2]).unwrap());
            let code = CloseCode::from(raw_code);
            let description = if payload.len() > 2 {
                Some(String::from_utf8_lossy(&payload[2..]).into())
            } else {
                None
            };
            Some(CloseReason { code, description })
        } else {
            None
        }
    }

    /// Generate binary representation
    pub fn write_message<B: AsRef<[u8]>>(
        dst: &mut BytesMut,
        pl: B,
        op: OpCode,
        rsv_bits: RsvBits,
        fin: bool,
        mask: bool,
    ) {
        let payload = pl.as_ref();
        let fin_bits = if fin { 0x80 } else { 0x00 };
        let rsv_bits = rsv_bits.bits() << 4;

        let one: u8 = fin_bits | rsv_bits | Into::<u8>::into(op);
        let payload_len = payload.len();
        let (two, p_len) = if mask {
            (0x80, payload_len + 4)
        } else {
            (0, payload_len)
        };

        if payload_len < 126 {
            dst.reserve(p_len + 2);
            dst.put_slice(&[one, two | payload_len as u8]);
        } else if payload_len <= 65_535 {
            dst.reserve(p_len + 4);
            dst.put_slice(&[one, two | 126]);
            dst.put_u16(payload_len as u16);
        } else {
            dst.reserve(p_len + 10);
            dst.put_slice(&[one, two | 127]);
            dst.put_u64(payload_len as u64);
        };

        if mask {
            let mask = rand::random::<[u8; 4]>();
            dst.put_slice(mask.as_ref());
            dst.put_slice(payload.as_ref());
            let pos = dst.len() - payload_len;
            apply_mask(&mut dst[pos..], mask);
        } else {
            dst.put_slice(payload.as_ref());
        }
    }

    /// Create a new Close control frame.
    #[inline]
    pub fn write_close(
        dst: &mut BytesMut,
        reason: Option<CloseReason>,
        rsv_bits: RsvBits,
        mask: bool,
    ) {
        let payload = match reason {
            None => Vec::new(),
            Some(reason) => {
                let mut payload = Into::<u16>::into(reason.code).to_be_bytes().to_vec();
                if let Some(description) = reason.description {
                    payload.extend(description.as_bytes());
                }
                payload
            }
        };

        Parser::write_message(dst, payload, OpCode::Close, rsv_bits, true, mask)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    struct F {
        finished: bool,
        opcode: OpCode,
        rsv_bits: RsvBits,
        payload: Bytes,
    }

    fn is_none(
        frm: &Result<Option<(bool, OpCode, RsvBits, Option<BytesMut>)>, ProtocolError>,
    ) -> bool {
        matches!(*frm, Ok(None))
    }

    fn extract(frm: Result<Option<(bool, OpCode, RsvBits, Option<BytesMut>)>, ProtocolError>) -> F {
        match frm {
            Ok(Some((finished, opcode, rsv_bits, payload))) => F {
                finished,
                opcode,
                rsv_bits,
                payload: payload
                    .map(|b| b.freeze())
                    .unwrap_or_else(|| Bytes::from("")),
            },
            _ => unreachable!("error"),
        }
    }

    #[test]
    fn test_parse() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0001u8][..]);
        assert!(is_none(&Parser::parse(&mut buf, false, 1024)));

        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0001u8][..]);
        buf.extend(b"1");

        let frame = extract(Parser::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1"[..]);

        let mut buf = BytesMut::from(&[0b1111_0001u8, 0b0000_0001u8][..]);
        buf.extend(b"2");

        let frame = extract(Parser::parse(&mut buf, false, 1024));
        assert!(frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"2"[..]);
        assert!(frame.rsv_bits.contains(RsvBits::RSV1));
        assert!(frame.rsv_bits.contains(RsvBits::RSV2));
        assert!(frame.rsv_bits.contains(RsvBits::RSV3));
    }

    #[test]
    fn test_parse_length0() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0000u8][..]);
        let frame = extract(Parser::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn test_parse_length2() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 126u8][..]);
        assert!(is_none(&Parser::parse(&mut buf, false, 1024)));

        let mut buf = BytesMut::from(&[0b0000_0001u8, 126u8][..]);
        buf.extend(&[0u8, 4u8][..]);
        buf.extend(b"1234");

        let frame = extract(Parser::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1234"[..]);
    }

    #[test]
    fn test_parse_length4() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 127u8][..]);
        assert!(is_none(&Parser::parse(&mut buf, false, 1024)));

        let mut buf = BytesMut::from(&[0b0000_0001u8, 127u8][..]);
        buf.extend(&[0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 4u8][..]);
        buf.extend(b"1234");

        let frame = extract(Parser::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1234"[..]);
    }

    #[test]
    fn test_parse_frame_mask() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b1000_0001u8][..]);
        buf.extend(b"0001");
        buf.extend(b"1");

        assert!(Parser::parse(&mut buf, false, 1024).is_err());

        let frame = extract(Parser::parse(&mut buf, true, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload, Bytes::from(vec![1u8]));
    }

    #[test]
    fn test_parse_frame_no_mask() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0001u8][..]);
        buf.extend([1u8]);

        assert!(Parser::parse(&mut buf, true, 1024).is_err());

        let frame = extract(Parser::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload, Bytes::from(vec![1u8]));
    }

    #[test]
    fn test_parse_frame_max_size() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0010u8][..]);
        buf.extend([1u8, 1u8]);

        assert!(Parser::parse(&mut buf, true, 1).is_err());

        if let Err(ProtocolError::Overflow) = Parser::parse(&mut buf, false, 0) {
        } else {
            unreachable!("error");
        }
    }

    #[test]
    fn test_parse_frame_max_size_recoverability() {
        let mut buf = BytesMut::new();
        // The first text frame with length == 2, payload doesn't matter.
        buf.extend([0b0000_0001u8, 0b0000_0010u8, 0b0000_0000u8, 0b0000_0000u8]);
        // Next binary frame with length == 2 and payload == `[0x1111_1111u8, 0x1111_1111u8]`.
        buf.extend([0b0000_0010u8, 0b0000_0010u8, 0b1111_1111u8, 0b1111_1111u8]);

        assert_eq!(buf.len(), 8);
        assert!(matches!(
            Parser::parse(&mut buf, false, 1),
            Err(ProtocolError::Overflow)
        ));
        assert_eq!(buf.len(), 4);
        let frame = extract(Parser::parse(&mut buf, false, 2));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Binary);
        assert_eq!(
            frame.payload,
            Bytes::from(vec![0b1111_1111u8, 0b1111_1111u8])
        );
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_ping_frame() {
        let mut buf = BytesMut::new();
        Parser::write_message(
            &mut buf,
            Vec::from("data"),
            OpCode::Ping,
            RsvBits::empty(),
            true,
            false,
        );

        let mut v = vec![137u8, 4u8];
        v.extend(b"data");
        assert_eq!(&buf[..], &v[..]);
    }

    #[test]
    fn test_pong_frame() {
        let mut buf = BytesMut::new();
        Parser::write_message(
            &mut buf,
            Vec::from("data"),
            OpCode::Pong,
            RsvBits::empty(),
            true,
            false,
        );

        let mut v = vec![138u8, 4u8];
        v.extend(b"data");
        assert_eq!(&buf[..], &v[..]);
    }

    #[test]
    fn test_close_frame() {
        let mut buf = BytesMut::new();
        let reason = (CloseCode::Normal, "data");
        Parser::write_close(&mut buf, Some(reason.into()), RsvBits::empty(), false);

        let mut v = vec![136u8, 6u8, 3u8, 232u8];
        v.extend(b"data");
        assert_eq!(&buf[..], &v[..]);
    }

    #[test]
    fn test_empty_close_frame() {
        let mut buf = BytesMut::new();
        Parser::write_close(&mut buf, None, RsvBits::empty(), false);
        assert_eq!(&buf[..], &vec![0x88, 0x00][..]);
    }
}

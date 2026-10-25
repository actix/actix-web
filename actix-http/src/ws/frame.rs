use std::{cmp::min, io, str};

use bytes::{Buf, BufMut, BytesMut};

use super::{
    mask::apply_mask,
    proto::{CloseCode, CloseReason, OpCode},
    ProtocolError,
};

/// A struct representing a WebSocket frame.
#[derive(Debug)]
pub struct Parser;

/// FIN bitmask (RFC 6455 §5.2).
const FIN_MASK: u8 = 0x80;

/// RSV1, RSV2, and RSV3 bitmask (RFC 6455 §5.2).
const RSV_MASK: u8 = 0b0111_0000;

/// OpCode 4-bit nibble mask (RFC 6455 §5.2).
const OPCODE_MASK: u8 = 0x0F;

/// MASK bit bitmask (RFC 6455 §5.2).
const MASK_BIT: u8 = 0x80;

/// 7-bit payload length bitmask (RFC 6455 §5.2).
const PAYLOAD_LEN_MASK: u8 = 0x7F;

/// Extended payload length indicator for 16-bit length (RFC 6455 §5.2).
const EXT_LEN_U16: u8 = 126;

/// Extended payload length indicator for 64-bit length (RFC 6455 §5.2).
const EXT_LEN_U64: u8 = 127;

/// Maximum payload length allowed for control frames (RFC 6455 §5.5).
const MAX_CONTROL_FRAME_PAYLOAD: usize = 125;

/// Maximum payload length representable by a 16-bit extended length (RFC 6455 §5.2).
const U16_PAYLOAD_MAX: usize = u16::MAX as usize;

/// Maximum legal payload length for a 64-bit extended length (RFC 6455 §5.2).
const U64_PAYLOAD_MAX: u64 = i64::MAX as u64;

/// Client masking key length in bytes (RFC 6455 §5.3).
const MASK_LEN: usize = 4;

/// Convert a payload length to `usize`, failing if it does not fit.
///
/// A `usize` cannot represent every 63-bit length on 32-bit targets, where the conversion must
/// fail rather than truncate. Truncating would make the parser report a shorter frame and treat
/// the remaining bytes as the start of the next one.
fn payload_len_to_usize(len: u64) -> Result<usize, ProtocolError> {
    usize::try_from(len).map_err(|_| ProtocolError::Overflow)
}

/// Whether the most significant bit of a 64-bit payload length is set.
///
/// RFC 6455 §5.2 defines the payload length as an unsigned 63-bit integer, so the most
/// significant bit must be zero.
fn payload_len_msb_set(len: u64) -> bool {
    len > U64_PAYLOAD_MAX
}

impl Parser {
    fn parse_metadata(
        src: &[u8],
        server: bool,
    ) -> Result<Option<(usize, bool, OpCode, usize, Option<[u8; 4]>)>, ProtocolError> {
        let chunk_len = src.len();

        let mut idx = 2;
        if chunk_len < 2 {
            return Ok(None);
        }

        let first = src[0];
        let second = src[1];
        let finished = first & FIN_MASK != 0;

        // RSV1, RSV2, and RSV3 must be zero unless a negotiated extension defines them.
        if first & RSV_MASK != 0 {
            // TODO(semver-major): use InvalidReservedBits
            return Err(ProtocolError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "Received a frame with non-zero reserved bits",
            )));
        }

        // check masking
        let masked = second & MASK_BIT != 0;
        if !masked && server {
            return Err(ProtocolError::UnmaskedFrame);
        } else if masked && !server {
            return Err(ProtocolError::MaskedFrame);
        }

        // Op code
        let opcode = OpCode::from(first & OPCODE_MASK);

        if let OpCode::Bad = opcode {
            return Err(ProtocolError::InvalidOpcode(first & OPCODE_MASK));
        }

        let len = second & PAYLOAD_LEN_MASK;
        let length = if len == EXT_LEN_U16 {
            if chunk_len < 4 {
                return Ok(None);
            }
            let len = usize::from(u16::from_be_bytes(
                TryFrom::try_from(&src[idx..idx + 2]).unwrap(),
            ));
            idx += 2;
            len
        } else if len == EXT_LEN_U64 {
            if chunk_len < 10 {
                return Ok(None);
            }
            let len = u64::from_be_bytes(TryFrom::try_from(&src[idx..idx + 8]).unwrap());
            idx += 8;

            if payload_len_msb_set(len) {
                // TODO(semver-major): use a variant that describes an invalid length, rather than
                //  the payload size limit that `Overflow` also documents.
                return Err(ProtocolError::Overflow);
            }

            payload_len_to_usize(len)?
        } else {
            len as usize
        };

        let mask = if server {
            if chunk_len < idx + MASK_LEN {
                return Ok(None);
            }

            let mask = TryFrom::try_from(&src[idx..idx + MASK_LEN]).unwrap();

            idx += MASK_LEN;

            Some(mask)
        } else {
            None
        };

        Ok(Some((idx, finished, opcode, length, mask)))
    }

    /// Parse the input stream into a frame.
    pub fn parse(
        src: &mut BytesMut,
        server: bool,
        max_size: usize,
    ) -> Result<Option<(bool, OpCode, Option<BytesMut>)>, ProtocolError> {
        // try to parse ws frame metadata
        let (idx, finished, opcode, length, mask) = match Parser::parse_metadata(src, server)? {
            None => return Ok(None),
            Some(res) => res,
        };

        // Live on 32-bit targets, where `u32::MAX` fits a `usize` and overflows this. On 64-bit
        // it cannot fire: overflowing needs a length above `u64::MAX - idx`, which has the most
        // significant bit set and is rejected earlier.
        let frame_len = match idx.checked_add(length) {
            Some(len) => len,
            None => return Err(ProtocolError::Overflow),
        };

        // not enough data
        if src.len() < frame_len {
            let min_length = min(length, max_size);
            let required_cap = match idx.checked_add(min_length) {
                Some(cap) => cap,
                None => return Err(ProtocolError::Overflow),
            };

            if src.capacity() < required_cap {
                src.reserve(required_cap - src.capacity());
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
            return Ok(Some((finished, opcode, None)));
        }

        let mut data = src.split_to(length);

        // control frames must have length <= 125 (RFC 6455 §5.5)
        match opcode {
            OpCode::Ping | OpCode::Pong | OpCode::Close if length > MAX_CONTROL_FRAME_PAYLOAD => {
                return Err(ProtocolError::InvalidLength(length));
            }
            _ => {}
        }

        // unmask
        if let Some(mask) = mask {
            apply_mask(&mut data, mask);
        }

        Ok(Some((finished, opcode, Some(data))))
    }

    /// Parse the payload of a close frame.
    ///
    /// This method preserves the historical behavior of accepting unknown status codes and
    /// replacing invalid UTF-8 in the reason. Use [`Parser::try_parse_close_payload`] when the
    /// payload must be validated.
    #[deprecated(
        since = "3.13.5",
        note = "Use `Parser::try_parse_close_payload` instead."
    )]
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

    /// Parse and validate the payload of a close frame.
    ///
    /// # Validation
    ///
    /// A close payload is either empty or contains a two-byte status code followed by an optional
    /// UTF-8 reason. This follows [RFC 6455 §5.5.1], with status code rules from [RFC 6455 §7.4]
    /// and UTF-8 error handling from [RFC 6455 §8.1].
    ///
    /// [RFC 6455 §5.5.1]: https://datatracker.ietf.org/doc/html/rfc6455#section-5.5.1
    /// [RFC 6455 §7.4]: https://datatracker.ietf.org/doc/html/rfc6455#section-7.4
    /// [RFC 6455 §8.1]: https://datatracker.ietf.org/doc/html/rfc6455#section-8.1
    pub fn try_parse_close_payload(payload: &[u8]) -> Result<Option<CloseReason>, ProtocolError> {
        // RFC 6455 §5.5.1 requires a two-byte status code when a close payload is not empty.
        if payload.len() == 1 {
            return Err(ProtocolError::InvalidLength(payload.len()));
        }

        if payload.len() >= 2 {
            let raw_code = u16::from_be_bytes(
                payload[..2]
                    .try_into()
                    .expect("Payload length should be checked before parsing"),
            );

            // RFC 6455 §7.4 reserves 1000-2999 for protocol-defined codes. Reject reserved and
            // undefined values while allowing registered codes (3000-3999) and private-use codes
            // (4000-4999).
            if !matches!(raw_code, 1000..=1003 | 1007..=1014 | 3000..=4999) {
                // TODO(semver-major): use this instead
                // return Err(ProtocolError::InvalidCloseCode(raw_code));
                return Err(ProtocolError::BadOpCode);
            }

            if payload.len() > 2 {
                // RFC 6455 §5.5.1 defines the remaining bytes as a UTF-8 reason. RFC 6455 §8.1
                // requires the connection to fail when data interpreted as UTF-8 is invalid.
                str::from_utf8(&payload[2..]).map_err(|_| {
                    // TODO(semver-major): use this instead
                    // ProtocolError::InvalidCloseReason
                    ProtocolError::BadOpCode
                })?;
            }
        }

        #[expect(deprecated)]
        Ok(Self::parse_close_payload(payload))
    }

    /// Generate binary representation
    pub fn write_message<B: AsRef<[u8]>>(
        dst: &mut BytesMut,
        pl: B,
        op: OpCode,
        fin: bool,
        mask: bool,
    ) {
        let payload = pl.as_ref();
        let one = if fin {
            FIN_MASK | u8::from(op)
        } else {
            u8::from(op)
        };
        let payload_len = payload.len();
        let (two, p_len) = if mask {
            (MASK_BIT, payload_len + MASK_LEN)
        } else {
            (0, payload_len)
        };

        if payload_len < EXT_LEN_U16 as usize {
            dst.reserve(p_len + 2);
            dst.put_slice(&[one, two | payload_len as u8]);
        } else if payload_len <= U16_PAYLOAD_MAX {
            dst.reserve(p_len + 4);
            dst.put_slice(&[one, two | EXT_LEN_U16]);
            dst.put_u16(payload_len as u16);
        } else {
            dst.reserve(p_len + 10);
            dst.put_slice(&[one, two | EXT_LEN_U64]);
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
    pub fn write_close(dst: &mut BytesMut, reason: Option<CloseReason>, mask: bool) {
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

        Parser::write_message(dst, payload, OpCode::Close, true, mask)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    struct F {
        finished: bool,
        opcode: OpCode,
        payload: Bytes,
    }

    fn is_none(frm: &Result<Option<(bool, OpCode, Option<BytesMut>)>, ProtocolError>) -> bool {
        matches!(*frm, Ok(None))
    }

    fn extract(frm: Result<Option<(bool, OpCode, Option<BytesMut>)>, ProtocolError>) -> F {
        match frm {
            Ok(Some((finished, opcode, payload))) => F {
                finished,
                opcode,
                payload: payload
                    .map(|b| b.freeze())
                    .unwrap_or_else(|| Bytes::from("")),
            },
            _ => unreachable!("error"),
        }
    }

    // RFC 6455 §5.2: the most significant bit of a 64-bit payload length MUST be 0.
    #[test]
    fn test_parse_rejects_64_bit_length_with_msb_set() {
        let mut buf = BytesMut::from(&[0x81u8, 127][..]);
        buf.extend_from_slice(&(U64_PAYLOAD_MAX + 1).to_be_bytes());

        assert!(
            matches!(
                Parser::parse(&mut buf, false, 1024),
                Err(ProtocolError::Overflow)
            ),
            "a 64-bit payload length with the MSB set must be rejected, not treated as incomplete"
        );
        // The rejection must not consume the header, or the next parse would start mid-frame.
        assert_eq!(buf.len(), 10);
    }

    // Pins the accept side of the boundary through `parse`: the largest legal length must be
    // treated as incomplete, not rejected. Only a 64-bit `usize` can represent it.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn test_parse_accepts_largest_legal_64_bit_length() {
        let mut buf = BytesMut::from(&[0x81u8, 127][..]);
        buf.extend_from_slice(&U64_PAYLOAD_MAX.to_be_bytes());

        // `max_size` is deliberately far below the declared length: an incomplete frame returns
        // before the size check, so the header is accepted as incomplete. It also keeps the
        // parser from reserving the declared length, which a large `max_size` would attempt.
        assert!(
            is_none(&Parser::parse(&mut buf, false, 1024)),
            "the largest legal 64-bit length must be incomplete, not rejected"
        );
        // The header must stay buffered while the payload is awaited.
        assert_eq!(buf.len(), 10);
    }

    // The most-significant-bit rule itself, on every target. On 32-bit the conversion rejects every
    // such length anyway, so the call site is not observable through `parse`; pin the predicate.
    #[test]
    fn test_payload_len_msb_set() {
        assert!(!payload_len_msb_set(0));
        assert!(!payload_len_msb_set(U64_PAYLOAD_MAX));
        assert!(payload_len_msb_set(1 << 63));
        assert!(payload_len_msb_set(u64::MAX));
    }

    // A declared 2^32 does not fit a 32-bit `usize`, and truncating it would accept an empty frame
    // and leave the trailing bytes to be read as the next frame.
    #[cfg(target_pointer_width = "32")]
    #[test]
    fn test_parse_rejects_64_bit_length_beyond_addressable_range() {
        let mut buf = BytesMut::from(&[0x81u8, 127][..]);
        buf.extend_from_slice(&0x0000_0001_0000_0000u64.to_be_bytes());
        buf.extend_from_slice(b"ABCD");

        assert!(
            matches!(
                Parser::parse(&mut buf, false, 1024),
                Err(ProtocolError::Overflow)
            ),
            "a length that does not fit in usize must be rejected, not truncated to 0"
        );
        assert_eq!(buf.len(), 14);
    }

    // `u32::MAX` fits a 32-bit `usize` and its most significant bit is clear, so neither the
    // most-significant-bit check nor the conversion rejects it and only the `checked_add` guard in
    // `parse` can.
    #[cfg(target_pointer_width = "32")]
    #[test]
    fn test_parse_rejects_length_that_overflows_frame_len() {
        let mut buf = BytesMut::from(&[0x81u8, 127][..]);
        buf.extend_from_slice(&(u32::MAX as u64).to_be_bytes());

        assert!(matches!(
            Parser::parse(&mut buf, false, 1024),
            Err(ProtocolError::Overflow)
        ));
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
    fn test_parse_frame_with_rsv1_set() {
        // https://github.com/actix/actix-web/issues/1579

        // Final, masked text frame with RSV1 set and no negotiated extension.
        let mut buf = BytesMut::from(
            &[
                0b1100_0001u8, // FIN + RSV1 + text
                0b1000_0001u8, // MASK + payload length
                0,             // masking key byte 1
                0,             // masking key byte 2
                0,             // masking key byte 3
                0,             // masking key byte 4
                b'a',          // payload
            ][..],
        );

        Parser::parse(&mut buf, true, 1024)
            .expect_err("Should reject set RSV1 bit when no extension is negotiated");
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
        Parser::write_message(&mut buf, Vec::from("data"), OpCode::Ping, true, false);

        let mut v = vec![137u8, 4u8];
        v.extend(b"data");
        assert_eq!(&buf[..], &v[..]);
    }

    #[test]
    fn test_pong_frame() {
        let mut buf = BytesMut::new();
        Parser::write_message(&mut buf, Vec::from("data"), OpCode::Pong, true, false);

        let mut v = vec![138u8, 4u8];
        v.extend(b"data");
        assert_eq!(&buf[..], &v[..]);
    }

    #[test]
    fn test_close_frame() {
        let mut buf = BytesMut::new();
        let reason = (CloseCode::Normal, "data");
        Parser::write_close(&mut buf, Some(reason.into()), false);

        let mut v = vec![136u8, 6u8, 3u8, 232u8];
        v.extend(b"data");
        assert_eq!(&buf[..], &v[..]);
    }

    #[test]
    fn test_empty_close_frame() {
        let mut buf = BytesMut::new();
        Parser::write_close(&mut buf, None, false);
        assert_eq!(&buf[..], &vec![0x88, 0x00][..]);
    }

    #[test]
    fn try_parse_close_payload_validates_payload() {
        assert!(matches!(
            Parser::try_parse_close_payload(&[0x03, 0xe8, 0xff]).unwrap_err(),
            ProtocolError::BadOpCode
        ));
        assert!(matches!(
            Parser::try_parse_close_payload(&[0, 0]).unwrap_err(),
            ProtocolError::BadOpCode
        ));
    }

    #[test]
    fn test_oversized_control_frames() {
        // Ping frame with 126 bytes payload (exceeding 125 limit)
        let mut buf = BytesMut::from(&[0x89u8, 126u8, 0x00, 126][..]);
        buf.extend(vec![0u8; 126]);
        assert!(matches!(
            Parser::parse(&mut buf, false, 65536),
            Err(ProtocolError::InvalidLength(126))
        ));

        // Close frame with 126 bytes payload (exceeding 125 limit)
        let mut buf = BytesMut::from(&[0x88u8, 126u8, 0x00, 126][..]);
        buf.extend(vec![0u8; 126]);
        assert!(matches!(
            Parser::parse(&mut buf, false, 65536),
            Err(ProtocolError::InvalidLength(126))
        ));
    }

    // The payload length of the #3723 repro, whose MSB is set. That report was an unchecked
    // addition that overflowed; `checked_add` was added to fix it, and the most-significant-bit
    // check now rejects this input before either runs, so this pins the regression only.
    // The `checked_add` guard is covered separately by a 32-bit-only test.
    #[test]
    fn test_parse_rejects_msb_length_from_3723() {
        let buf: [u8; 14] = [
            0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xeb, 0x0e, 0x8f,
        ];
        let mut buf = BytesMut::from(&buf[..]);
        let result = Parser::parse(&mut buf, true, 65536);
        // Opcode 0x0a is Pong. The length is rejected in `parse_metadata` before the frame can be
        // buffered, so the header is reported as malformed rather than awaited.
        assert!(
            matches!(result, Err(ProtocolError::Overflow)),
            "a declared length above i64::MAX must be rejected outright"
        );
    }
}

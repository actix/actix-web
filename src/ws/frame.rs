use byteorder::{BigEndian, ByteOrder, NetworkEndian};
use bytes::{BufMut, Bytes, BytesMut};
use futures::{Async, Poll, Stream};
use rand;
use std::iter::FromIterator;
use std::{fmt, mem, ptr};

use body::Binary;
use error::PayloadError;
use payload::PayloadHelper;

use ws::ProtocolError;
use ws::mask::apply_mask;
use ws::proto::{CloseCode, CloseReason, OpCode};

/// A struct representing a `WebSocket` frame.
#[derive(Debug)]
pub struct Frame {
    finished: bool,
    opcode: OpCode,
    payload: Binary,
}

impl Frame {
    /// Destruct frame
    pub fn unpack(self) -> (bool, OpCode, Binary) {
        (self.finished, self.opcode, self.payload)
    }

    /// Create a new Close control frame.
    #[inline]
    pub fn close(reason: Option<CloseReason>, genmask: bool) -> Binary {
	    let payload:Vec<u8> = match reason {
		    None => Vec::new(),
		    Some(reason) => {
			    let mut code_bytes = [0; 2];
			    NetworkEndian::write_u16(&mut code_bytes, reason.code.into());

			    let mut payload = Vec::from(&code_bytes[..]);
			    if let Some(description) = reason.description{
				    payload.extend(description.as_bytes());
			    }
			    payload
		    }
	    };

	    Frame::message(payload, OpCode::Close, true, genmask)
    }

    #[cfg_attr(feature = "cargo-clippy", allow(type_complexity))]
    fn read_copy_md<S>(
        pl: &mut PayloadHelper<S>, server: bool, max_size: usize
    ) -> Poll<Option<(usize, bool, OpCode, usize, Option<u32>)>, ProtocolError>
    where
        S: Stream<Item = Bytes, Error = PayloadError>,
    {
        let mut idx = 2;
        let buf = match pl.copy(2)? {
            Async::Ready(Some(buf)) => buf,
            Async::Ready(None) => return Ok(Async::Ready(None)),
            Async::NotReady => return Ok(Async::NotReady),
        };
        let first = buf[0];
        let second = buf[1];
        let finished = first & 0x80 != 0;

        // check masking
        let masked = second & 0x80 != 0;
        if !masked && server {
            return Err(ProtocolError::UnmaskedFrame);
        } else if masked && !server {
            return Err(ProtocolError::MaskedFrame);
        }

        // Op code
        let opcode = OpCode::from(first & 0x0F);

        if let OpCode::Bad = opcode {
            return Err(ProtocolError::InvalidOpcode(first & 0x0F));
        }

        let len = second & 0x7F;
        let length = if len == 126 {
            let buf = match pl.copy(4)? {
                Async::Ready(Some(buf)) => buf,
                Async::Ready(None) => return Ok(Async::Ready(None)),
                Async::NotReady => return Ok(Async::NotReady),
            };
            let len = NetworkEndian::read_uint(&buf[idx..], 2) as usize;
            idx += 2;
            len
        } else if len == 127 {
            let buf = match pl.copy(10)? {
                Async::Ready(Some(buf)) => buf,
                Async::Ready(None) => return Ok(Async::Ready(None)),
                Async::NotReady => return Ok(Async::NotReady),
            };
            let len = NetworkEndian::read_uint(&buf[idx..], 8) as usize;
            idx += 8;
            len
        } else {
            len as usize
        };

        // check for max allowed size
        if length > max_size {
            return Err(ProtocolError::Overflow);
        }

        let mask = if server {
            let buf = match pl.copy(idx + 4)? {
                Async::Ready(Some(buf)) => buf,
                Async::Ready(None) => return Ok(Async::Ready(None)),
                Async::NotReady => return Ok(Async::NotReady),
            };

            let mask: &[u8] = &buf[idx..idx + 4];
            let mask_u32: u32 =
                unsafe { ptr::read_unaligned(mask.as_ptr() as *const u32) };
            idx += 4;
            Some(mask_u32)
        } else {
            None
        };

        Ok(Async::Ready(Some((
            idx,
            finished,
            opcode,
            length,
            mask,
        ))))
    }

    fn read_chunk_md(
        chunk: &[u8], server: bool, max_size: usize
    ) -> Poll<(usize, bool, OpCode, usize, Option<u32>), ProtocolError> {
        let chunk_len = chunk.len();

        let mut idx = 2;
        if chunk_len < 2 {
            return Ok(Async::NotReady);
        }

        let first = chunk[0];
        let second = chunk[1];
        let finished = first & 0x80 != 0;

        // check masking
        let masked = second & 0x80 != 0;
        if !masked && server {
            return Err(ProtocolError::UnmaskedFrame);
        } else if masked && !server {
            return Err(ProtocolError::MaskedFrame);
        }

        // Op code
        let opcode = OpCode::from(first & 0x0F);

        if let OpCode::Bad = opcode {
            return Err(ProtocolError::InvalidOpcode(first & 0x0F));
        }

        let len = second & 0x7F;
        let length = if len == 126 {
            if chunk_len < 4 {
                return Ok(Async::NotReady);
            }
            let len = NetworkEndian::read_uint(&chunk[idx..], 2) as usize;
            idx += 2;
            len
        } else if len == 127 {
            if chunk_len < 10 {
                return Ok(Async::NotReady);
            }
            let len = NetworkEndian::read_uint(&chunk[idx..], 8) as usize;
            idx += 8;
            len
        } else {
            len as usize
        };

        // check for max allowed size
        if length > max_size {
            return Err(ProtocolError::Overflow);
        }

        let mask = if server {
            if chunk_len < idx + 4 {
                return Ok(Async::NotReady);
            }

            let mask: &[u8] = &chunk[idx..idx + 4];
            let mask_u32: u32 =
                unsafe { ptr::read_unaligned(mask.as_ptr() as *const u32) };
            idx += 4;
            Some(mask_u32)
        } else {
            None
        };

        Ok(Async::Ready((idx, finished, opcode, length, mask)))
    }

    /// Parse the input stream into a frame.
    pub fn parse<S>(
        pl: &mut PayloadHelper<S>, server: bool, max_size: usize
    ) -> Poll<Option<Frame>, ProtocolError>
    where
        S: Stream<Item = Bytes, Error = PayloadError>,
    {
        // try to parse ws frame md from one chunk
        let result = match pl.get_chunk()? {
            Async::NotReady => return Ok(Async::NotReady),
            Async::Ready(None) => return Ok(Async::Ready(None)),
            Async::Ready(Some(chunk)) => Frame::read_chunk_md(chunk, server, max_size)?,
        };

        let (idx, finished, opcode, length, mask) = match result {
            // we may need to join several chunks
            Async::NotReady => match Frame::read_copy_md(pl, server, max_size)? {
                Async::Ready(Some(item)) => item,
                Async::NotReady => return Ok(Async::NotReady),
                Async::Ready(None) => return Ok(Async::Ready(None)),
            },
            Async::Ready(item) => item,
        };

        match pl.can_read(idx + length)? {
            Async::Ready(Some(true)) => (),
            Async::Ready(None) => return Ok(Async::Ready(None)),
            Async::Ready(Some(false)) | Async::NotReady => return Ok(Async::NotReady),
        }

        // remove prefix
        pl.drop_payload(idx);

        // no need for body
        if length == 0 {
            return Ok(Async::Ready(Some(Frame {
                finished,
                opcode,
                payload: Binary::from(""),
            })));
        }

        let data = match pl.read_exact(length)? {
            Async::Ready(Some(buf)) => buf,
            Async::Ready(None) => return Ok(Async::Ready(None)),
            Async::NotReady => panic!(),
        };

        // control frames must have length <= 125
        match opcode {
            OpCode::Ping | OpCode::Pong if length > 125 => {
                return Err(ProtocolError::InvalidLength(length))
            }
            OpCode::Close if length > 125 => {
                debug!("Received close frame with payload length exceeding 125. Morphing to protocol close frame.");
                return Ok(Async::Ready(Some(Frame::default())));
            }
            _ => (),
        }

        // unmask
        if let Some(mask) = mask {
            #[allow(mutable_transmutes)]
            let p: &mut [u8] = unsafe {
                let ptr: &[u8] = &data;
                mem::transmute(ptr)
            };
            apply_mask(p, mask);
        }

        Ok(Async::Ready(Some(Frame {
            finished,
            opcode,
            payload: data.into(),
        })))
    }

    /// Parse the payload of a close frame.
    pub fn parse_close_payload(payload: &Binary) -> Option<CloseReason> {
        if payload.len() >= 2 {
            let raw_code = NetworkEndian::read_uint(payload.as_ref(), 2) as u16;
            let code = CloseCode::from(raw_code);
            let description = if payload.len() > 2 {
                Some(String::from_utf8_lossy(&payload.as_ref()[2..]).into())
            } else {
                None
            };
            Some(CloseReason { code, description })
        } else {
            None
        }
    }

    /// Generate binary representation
    pub fn message<B: Into<Binary>>(
        data: B, code: OpCode, finished: bool, genmask: bool
    ) -> Binary {
        let payload = data.into();
        let one: u8 = if finished {
            0x80 | Into::<u8>::into(code)
        } else {
            code.into()
        };
        let payload_len = payload.len();
        let (two, p_len) = if genmask {
            (0x80, payload_len + 4)
        } else {
            (0, payload_len)
        };

        let mut buf = if payload_len < 126 {
            let mut buf = BytesMut::with_capacity(p_len + 2);
            buf.put_slice(&[one, two | payload_len as u8]);
            buf
        } else if payload_len <= 65_535 {
            let mut buf = BytesMut::with_capacity(p_len + 4);
            buf.put_slice(&[one, two | 126]);
            {
                let buf_mut = unsafe { buf.bytes_mut() };
                BigEndian::write_u16(&mut buf_mut[..2], payload_len as u16);
            }
            unsafe { buf.advance_mut(2) };
            buf
        } else {
            let mut buf = BytesMut::with_capacity(p_len + 10);
            buf.put_slice(&[one, two | 127]);
            {
                let buf_mut = unsafe { buf.bytes_mut() };
                BigEndian::write_u64(&mut buf_mut[..8], payload_len as u64);
            }
            unsafe { buf.advance_mut(8) };
            buf
        };

        if genmask {
            let mask = rand::random::<u32>();
            unsafe {
                {
                    let buf_mut = buf.bytes_mut();
                    *(buf_mut as *mut _ as *mut u32) = mask;
                    buf_mut[4..payload_len + 4].copy_from_slice(payload.as_ref());
                    apply_mask(&mut buf_mut[4..], mask);
                }
                buf.advance_mut(payload_len + 4);
            }
            buf.into()
        } else {
            buf.put_slice(payload.as_ref());
            buf.into()
        }
    }
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            finished: true,
            opcode: OpCode::Close,
            payload: Binary::from(&b""[..]),
        }
    }
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "
<FRAME>
    final: {}
    opcode: {}
    payload length: {}
    payload: 0x{}
</FRAME>",
            self.finished,
            self.opcode,
            self.payload.len(),
            self.payload
                .as_ref()
                .iter()
                .map(|byte| format!("{:x}", byte))
                .collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::once;

    fn is_none(frm: &Poll<Option<Frame>, ProtocolError>) -> bool {
        match *frm {
            Ok(Async::Ready(None)) => true,
            _ => false,
        }
    }

    fn extract(frm: Poll<Option<Frame>, ProtocolError>) -> Frame {
        match frm {
            Ok(Async::Ready(Some(frame))) => frame,
            _ => unreachable!("error"),
        }
    }

    #[test]
    fn test_parse() {
        let mut buf = PayloadHelper::new(once(Ok(BytesMut::from(
            &[0b0000_0001u8, 0b0000_0001u8][..],
        ).freeze())));
        assert!(is_none(&Frame::parse(&mut buf, false, 1024)));

        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0001u8][..]);
        buf.extend(b"1");
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));

        let frame = extract(Frame::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1"[..]);
    }

    #[test]
    fn test_parse_length0() {
        let buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0000u8][..]);
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));

        let frame = extract(Frame::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn test_parse_length2() {
        let buf = BytesMut::from(&[0b0000_0001u8, 126u8][..]);
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));
        assert!(is_none(&Frame::parse(&mut buf, false, 1024)));

        let mut buf = BytesMut::from(&[0b0000_0001u8, 126u8][..]);
        buf.extend(&[0u8, 4u8][..]);
        buf.extend(b"1234");
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));

        let frame = extract(Frame::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1234"[..]);
    }

    #[test]
    fn test_parse_length4() {
        let buf = BytesMut::from(&[0b0000_0001u8, 127u8][..]);
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));
        assert!(is_none(&Frame::parse(&mut buf, false, 1024)));

        let mut buf = BytesMut::from(&[0b0000_0001u8, 127u8][..]);
        buf.extend(&[0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 4u8][..]);
        buf.extend(b"1234");
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));

        let frame = extract(Frame::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1234"[..]);
    }

    #[test]
    fn test_parse_frame_mask() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b1000_0001u8][..]);
        buf.extend(b"0001");
        buf.extend(b"1");
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));

        assert!(Frame::parse(&mut buf, false, 1024).is_err());

        let frame = extract(Frame::parse(&mut buf, true, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload, vec![1u8].into());
    }

    #[test]
    fn test_parse_frame_no_mask() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0001u8][..]);
        buf.extend(&[1u8]);
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));

        assert!(Frame::parse(&mut buf, true, 1024).is_err());

        let frame = extract(Frame::parse(&mut buf, false, 1024));
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload, vec![1u8].into());
    }

    #[test]
    fn test_parse_frame_max_size() {
        let mut buf = BytesMut::from(&[0b0000_0001u8, 0b0000_0010u8][..]);
        buf.extend(&[1u8, 1u8]);
        let mut buf = PayloadHelper::new(once(Ok(buf.freeze())));

        assert!(Frame::parse(&mut buf, true, 1).is_err());

        if let Err(ProtocolError::Overflow) = Frame::parse(&mut buf, false, 0) {
        } else {
            unreachable!("error");
        }
    }

    #[test]
    fn test_ping_frame() {
        let frame = Frame::message(Vec::from("data"), OpCode::Ping, true, false);

        let mut v = vec![137u8, 4u8];
        v.extend(b"data");
        assert_eq!(frame, v.into());
    }

    #[test]
    fn test_pong_frame() {
        let frame = Frame::message(Vec::from("data"), OpCode::Pong, true, false);

        let mut v = vec![138u8, 4u8];
        v.extend(b"data");
        assert_eq!(frame, v.into());
    }

	#[test]
	fn test_close_frame() {
		let reason = (CloseCode::Normal, "data");
		let frame = Frame::close(Some(reason.into()), false);

		let mut v = vec![136u8, 6u8, 3u8, 232u8];
		v.extend(b"data");
		assert_eq!(frame, v.into());
	}

	#[test]
	fn test_empty_close_frame() {
		let frame = Frame::close(None, false);
		assert_eq!(frame, vec![0x88, 0x00].into());
	}
}

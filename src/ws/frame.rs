use std::{fmt, mem};
use std::io::{Write, Error, ErrorKind};
use std::iter::FromIterator;
use bytes::BytesMut;

use body::Binary;
use ws::proto::{OpCode, CloseCode};
use ws::mask::apply_mask;

/// A struct representing a `WebSocket` frame.
#[derive(Debug)]
pub(crate) struct Frame {
    finished: bool,
    rsv1: bool,
    rsv2: bool,
    rsv3: bool,
    opcode: OpCode,
    mask: Option<[u8; 4]>,
    payload: Binary,
}

impl Frame {

    /// Destruct frame
    pub fn unpack(self) -> (bool, OpCode, Binary) {
        (self.finished, self.opcode, self.payload)
    }

    /// Get the length of the frame.
    /// This is the length of the header + the length of the payload.
    #[inline]
    pub fn len(&self) -> usize {
        let mut header_length = 2;
        let payload_len = self.payload.len();
        if payload_len > 125 {
            if payload_len <= u16::max_value() as usize {
                header_length += 2;
            } else {
                header_length += 8;
            }
        }

        if self.mask.is_some() {
            header_length += 4;
        }

        header_length + payload_len
    }

    /// Create a new data frame.
    #[inline]
    pub fn message<B: Into<Binary>>(data: B, code: OpCode, finished: bool) -> Frame {
        Frame {
            finished: finished,
            opcode: code,
            payload: data.into(),
            .. Frame::default()
        }
    }

    /// Create a new Close control frame.
    #[inline]
    pub fn close(code: CloseCode, reason: &str) -> Frame {
        let raw: [u8; 2] = unsafe {
            let u: u16 = code.into();
            mem::transmute(u.to_be())
        };

        let payload = if let CloseCode::Empty = code {
            Vec::new()
        } else {
            Vec::from_iter(
                raw[..].iter()
                    .chain(reason.as_bytes().iter())
                    .cloned())
        };

        Frame {
            payload: payload.into(),
            .. Frame::default()
        }
    }

    /// Parse the input stream into a frame.
    pub fn parse(buf: &mut BytesMut) -> Result<Option<Frame>, Error> {
        let mut idx = 2;

        let (frame, length) = {
            let mut size = buf.len();

            if size < 2 {
                return Ok(None)
            }
            let mut head = [0u8; 2];
            size -= 2;
            head.copy_from_slice(&buf[..2]);

            trace!("Parsed headers {:?}", head);

            let first = head[0];
            let second = head[1];
            trace!("First: {:b}", first);
            trace!("Second: {:b}", second);

            let finished = first & 0x80 != 0;

            let rsv1 = first & 0x40 != 0;
            let rsv2 = first & 0x20 != 0;
            let rsv3 = first & 0x10 != 0;

            let opcode = OpCode::from(first & 0x0F);
            trace!("Opcode: {:?}", opcode);

            let masked = second & 0x80 != 0;
            trace!("Masked: {:?}", masked);

            let mut header_length = 2;
            let mut length = u64::from(second & 0x7F);

            if length == 126 {
                if size < 2 {
                    return Ok(None)
                }
                let mut length_bytes = [0u8; 2];
                length_bytes.copy_from_slice(&buf[idx..idx+2]);
                size -= 2;
                idx += 2;

                length = u64::from(unsafe{
                    let mut wide: u16 = mem::transmute(length_bytes);
                    wide = u16::from_be(wide);
                    wide});
                header_length += 2;
            } else if length == 127 {
                if size < 8 {
                    return Ok(None)
                }
                let mut length_bytes = [0u8; 8];
                length_bytes.copy_from_slice(&buf[idx..idx+8]);
                size -= 8;
                idx += 8;

                unsafe { length = mem::transmute(length_bytes); }
                length = u64::from_be(length);
                header_length += 8;
            }
            trace!("Payload length: {}", length);

            let mask = if masked {
                let mut mask_bytes = [0u8; 4];
                if size < 4 {
                    return Ok(None)
                } else {
                    header_length += 4;
                    size -= 4;
                    mask_bytes.copy_from_slice(&buf[idx..idx+4]);
                    idx += 4;
                    Some(mask_bytes)
                }
            } else {
                None
            };

            let length = length as usize;
            if size < length {
                return Ok(None)
            }

            let mut data = Vec::with_capacity(length);
            if length > 0 {
                data.extend_from_slice(&buf[idx..idx+length]);
            }

            // Disallow bad opcode
            if let OpCode::Bad = opcode {
                return Err(
                    Error::new(
                        ErrorKind::Other,
                        format!("Encountered invalid opcode: {}", first & 0x0F)))
            }

            // control frames must have length <= 125
            match opcode {
                OpCode::Ping | OpCode::Pong if length > 125 => {
                    return Err(
                        Error::new(
                            ErrorKind::Other,
                            format!("Rejected WebSocket handshake.Received control frame with length: {}.", length)))
                }
                OpCode::Close if length > 125 => {
                    debug!("Received close frame with payload length exceeding 125. Morphing to protocol close frame.");
                    return Ok(Some(Frame::close(CloseCode::Protocol, "Received close frame with payload length exceeding 125.")))
                }
                _ => ()
            }

            // unmask
            if let Some(ref mask) = mask {
                apply_mask(&mut data, mask);
            }

            let frame = Frame {
                finished: finished,
                rsv1: rsv1,
                rsv2: rsv2,
                rsv3: rsv3,
                opcode: opcode,
                mask: mask,
                payload: data.into(),
            };

            (frame, header_length + length)
        };

        buf.split_to(length);
        Ok(Some(frame))
    }

    /// Write a frame out to a buffer
    pub fn format<W: Write>(&mut self, w: &mut W) -> Result<(), Error> {
        let mut one = 0u8;
        let code: u8 = self.opcode.into();
        if self.finished {
            one |= 0x80;
        }
        if self.rsv1 {
            one |= 0x40;
        }
        if self.rsv2 {
            one |= 0x20;
        }
        if self.rsv3 {
            one |= 0x10;
        }
        one |= code;

        let mut two = 0u8;

        if self.mask.is_some() {
            two |= 0x80;
        }

        if self.payload.len() < 126 {
            two |= self.payload.len() as u8;
            let headers = [one, two];
            w.write_all(&headers)?;
        } else if self.payload.len() <= 65_535 {
            two |= 126;
            let length_bytes: [u8; 2] = unsafe {
                let short = self.payload.len() as u16;
                mem::transmute(short.to_be())
            };
            let headers = [one, two, length_bytes[0], length_bytes[1]];
            w.write_all(&headers)?;
        } else {
            two |= 127;
            let length_bytes: [u8; 8] = unsafe {
                let long = self.payload.len() as u64;
                mem::transmute(long.to_be())
            };
            let headers = [
                one,
                two,
                length_bytes[0],
                length_bytes[1],
                length_bytes[2],
                length_bytes[3],
                length_bytes[4],
                length_bytes[5],
                length_bytes[6],
                length_bytes[7],
            ];
            w.write_all(&headers)?;
        }

        if self.mask.is_some() {
            let mask = self.mask.take().unwrap();
            let mut payload = Vec::from(self.payload.as_ref());
            apply_mask(&mut payload, &mask);
            w.write_all(&mask)?;
            w.write_all(payload.as_ref())?;
        } else {
            w.write_all(self.payload.as_ref())?;
        }
        Ok(())
    }
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            finished: true,
            rsv1: false,
            rsv2: false,
            rsv3: false,
            opcode: OpCode::Close,
            mask: None,
            payload: Binary::from(&b""[..]),
        }
    }
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
            "
<FRAME>
    final: {}
    reserved: {} {} {}
    opcode: {}
    length: {}
    payload length: {}
    payload: 0x{}
</FRAME>",
               self.finished,
               self.rsv1,
               self.rsv2,
               self.rsv3,
               self.opcode,
               // self.mask.map(|mask| format!("{:?}", mask)).unwrap_or("NONE".into()),
               self.len(),
               self.payload.len(),
               self.payload.as_ref().iter().map(
                   |byte| format!("{:x}", byte)).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        let mut buf = BytesMut::from(&[0b00000001u8, 0b00000001u8][..]);
        assert!(Frame::parse(&mut buf).unwrap().is_none());
        buf.extend(b"1");
        let frame = Frame::parse(&mut buf).unwrap().unwrap();
        println!("FRAME: {}", frame);
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1"[..]);
    }

    #[test]
    fn test_parse_length0() {
        let mut buf = BytesMut::from(&[0b00000001u8, 0b00000000u8][..]);
        let frame = Frame::parse(&mut buf).unwrap().unwrap();
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn test_parse_length2() {
        let mut buf = BytesMut::from(&[0b00000001u8, 126u8][..]);
        assert!(Frame::parse(&mut buf).unwrap().is_none());
        buf.extend(&[0u8, 4u8][..]);
        buf.extend(b"1234");

        let frame = Frame::parse(&mut buf).unwrap().unwrap();
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1234"[..]);
    }

    #[test]
    fn test_parse_length4() {
        let mut buf = BytesMut::from(&[0b00000001u8, 127u8][..]);
        assert!(Frame::parse(&mut buf).unwrap().is_none());
        buf.extend(&[0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 4u8][..]);
        buf.extend(b"1234");

        let frame = Frame::parse(&mut buf).unwrap().unwrap();
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload.as_ref(), &b"1234"[..]);
    }

    #[test]
    fn test_parse_frame_mask() {
        let mut buf = BytesMut::from(&[0b00000001u8, 0b10000001u8][..]);
        buf.extend(b"0001");
        buf.extend(b"1");

        let frame = Frame::parse(&mut buf).unwrap().unwrap();
        assert!(!frame.finished);
        assert_eq!(frame.opcode, OpCode::Text);
        assert_eq!(frame.payload, vec![1u8].into());
    }

    #[test]
    fn test_ping_frame() {
        let mut frame = Frame::message(Vec::from("data"), OpCode::Ping, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        let mut v = vec![137u8, 4u8];
        v.extend(b"data");
        assert_eq!(buf, v);
    }

    #[test]
    fn test_pong_frame() {
        let mut frame = Frame::message(Vec::from("data"), OpCode::Pong, true);
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        let mut v = vec![138u8, 4u8];
        v.extend(b"data");
        assert_eq!(buf, v);
    }

    #[test]
    fn test_close_frame() {
        let mut frame = Frame::close(CloseCode::Normal, "data");
        let mut buf = Vec::new();
        frame.format(&mut buf).unwrap();

        let mut v = vec![136u8, 6u8, 3u8, 232u8];
        v.extend(b"data");
        assert_eq!(buf, v);
    }
}

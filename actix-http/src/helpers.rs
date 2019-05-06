use std::{io, ptr};

use bytes::{BufMut, BytesMut};
use http::Version;

const DEC_DIGITS_LUT: &[u8] = b"0001020304050607080910111213141516171819\
      2021222324252627282930313233343536373839\
      4041424344454647484950515253545556575859\
      6061626364656667686970717273747576777879\
      8081828384858687888990919293949596979899";

pub(crate) const STATUS_LINE_BUF_SIZE: usize = 13;

pub(crate) fn write_status_line(version: Version, mut n: u16, bytes: &mut BytesMut) {
    let mut buf: [u8; STATUS_LINE_BUF_SIZE] = *b"HTTP/1.1 000 ";
    match version {
        Version::HTTP_2 => buf[5] = b'2',
        Version::HTTP_10 => buf[7] = b'0',
        Version::HTTP_09 => {
            buf[5] = b'0';
            buf[7] = b'9';
        }
        _ => (),
    }

    // decode least significant 2 chars
    let d1 = ((n % 100) * 2) as usize;
    n /= 100;

    buf[10] = DEC_DIGITS_LUT[d1];
    buf[11] = DEC_DIGITS_LUT[d1+1];

    // decode last (most significant) char
    buf[9] = (n as u8) + b'0';

    bytes.put_slice(&buf);
}

/// NOTE: bytes object has to contain enough space
pub fn write_content_length(mut n: usize, bytes: &mut BytesMut) {
    if n < 10 {
        let mut buf: [u8; 21] = *b"\r\ncontent-length: 0\r\n";
        buf[18] = (n as u8) + b'0';
        bytes.put_slice(&buf);
    } else if n < 100 {
        let mut buf: [u8; 22] = *b"\r\ncontent-length: 00\r\n";
        let d1 = n << 1;
        unsafe {
            ptr::copy_nonoverlapping(
                DEC_DIGITS_LUT.as_ptr().add(d1),
                buf.as_mut_ptr().offset(18),
                2,
            );
        }
        bytes.put_slice(&buf);
    } else if n < 1000 {
        let mut buf: [u8; 23] = *b"\r\ncontent-length: 000\r\n";
        // decode 2 more chars, if > 2 chars
        let d1 = (n % 100) << 1;
        n /= 100;
        unsafe {
            ptr::copy_nonoverlapping(
                DEC_DIGITS_LUT.as_ptr().add(d1),
                buf.as_mut_ptr().offset(19),
                2,
            )
        };

        // decode last 1
        buf[18] = (n as u8) + b'0';

        bytes.put_slice(&buf);
    } else {
        use std::fmt::Write;
        let _ignored = write!(bytes, "\r\ncontent-length: {}\r\n", n);
    }
}

pub(crate) struct Writer<'a>(pub &'a mut BytesMut);

impl<'a> io::Write for Writer<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_content_length() {
        let mut bytes = BytesMut::new();
        bytes.reserve(50);
        write_content_length(0, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 0\r\n"[..]);
        bytes.reserve(50);
        write_content_length(9, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 9\r\n"[..]);
        bytes.reserve(50);
        write_content_length(10, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 10\r\n"[..]);
        bytes.reserve(50);
        write_content_length(99, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 99\r\n"[..]);
        bytes.reserve(50);
        write_content_length(100, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 100\r\n"[..]);
        bytes.reserve(50);
        write_content_length(101, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 101\r\n"[..]);
        bytes.reserve(50);
        write_content_length(998, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 998\r\n"[..]);
        bytes.reserve(50);
        write_content_length(1000, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 1000\r\n"[..]);
        bytes.reserve(50);
        write_content_length(1001, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 1001\r\n"[..]);
        bytes.reserve(50);
        write_content_length(5909, &mut bytes);
        assert_eq!(bytes.take().freeze(), b"\r\ncontent-length: 5909\r\n"[..]);
    }
}

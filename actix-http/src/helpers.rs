use std::io;

use bytes::{BufMut, BytesMut};
use http::Version;

use crate::extensions::Extensions;

const DIGITS_START: u8 = b'0';

pub(crate) fn write_status_line(version: Version, n: u16, bytes: &mut BytesMut) {
    match version {
        Version::HTTP_11 => bytes.put_slice(b"HTTP/1.1 "),
        Version::HTTP_10 => bytes.put_slice(b"HTTP/1.0 "),
        Version::HTTP_09 => bytes.put_slice(b"HTTP/0.9 "),
        _ => {
            // other HTTP version handlers do not use this method
        }
    }

    let d100 = (n / 100) as u8;
    let d10 = ((n / 10) % 10) as u8;
    let d1 = (n % 10) as u8;

    bytes.put_u8(DIGITS_START + d100);
    bytes.put_u8(DIGITS_START + d10);
    bytes.put_u8(DIGITS_START + d1);

    // trailing space before reason
    bytes.put_u8(b' ');
}

/// NOTE: bytes object has to contain enough space
pub fn write_content_length(n: u64, bytes: &mut BytesMut) {
    if n == 0 {
        bytes.put_slice(b"\r\ncontent-length: 0\r\n");
        return;
    }

    let mut buf = itoa::Buffer::new();

    bytes.put_slice(b"\r\ncontent-length: ");
    bytes.put_slice(buf.format(n).as_bytes());
    bytes.put_slice(b"\r\n");
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

pub(crate) trait DataFactory {
    fn set(&self, ext: &mut Extensions);
}

pub(crate) struct Data<T>(pub(crate) T);

impl<T: Clone + 'static> DataFactory for Data<T> {
    fn set(&self, ext: &mut Extensions) {
        ext.insert(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::str::from_utf8;

    use super::*;

    #[test]
    fn test_status_line() {
        let mut bytes = BytesMut::new();
        bytes.reserve(50);
        write_status_line(Version::HTTP_11, 200, &mut bytes);
        assert_eq!(from_utf8(&bytes.split().freeze()).unwrap(), "HTTP/1.1 200 ");

        let mut bytes = BytesMut::new();
        bytes.reserve(50);
        write_status_line(Version::HTTP_09, 404, &mut bytes);
        assert_eq!(from_utf8(&bytes.split().freeze()).unwrap(), "HTTP/0.9 404 ");

        let mut bytes = BytesMut::new();
        bytes.reserve(50);
        write_status_line(Version::HTTP_09, 515, &mut bytes);
        assert_eq!(from_utf8(&bytes.split().freeze()).unwrap(), "HTTP/0.9 515 ");
    }

    #[test]
    fn test_write_content_length() {
        let mut bytes = BytesMut::new();
        bytes.reserve(50);
        write_content_length(0, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 0\r\n"[..]);
        bytes.reserve(50);
        write_content_length(9, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 9\r\n"[..]);
        bytes.reserve(50);
        write_content_length(10, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 10\r\n"[..]);
        bytes.reserve(50);
        write_content_length(99, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 99\r\n"[..]);
        bytes.reserve(50);
        write_content_length(100, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 100\r\n"[..]);
        bytes.reserve(50);
        write_content_length(101, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 101\r\n"[..]);
        bytes.reserve(50);
        write_content_length(998, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 998\r\n"[..]);
        bytes.reserve(50);
        write_content_length(1000, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 1000\r\n"[..]);
        bytes.reserve(50);
        write_content_length(1001, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 1001\r\n"[..]);
        bytes.reserve(50);
        write_content_length(5909, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 5909\r\n"[..]);
        bytes.reserve(50);
        write_content_length(9999, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 9999\r\n"[..]);
        bytes.reserve(50);
        write_content_length(10001, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 10001\r\n"[..]);
        bytes.reserve(50);
        write_content_length(59094, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 59094\r\n"[..]);
        bytes.reserve(50);
        write_content_length(99999, &mut bytes);
        assert_eq!(bytes.split().freeze(), b"\r\ncontent-length: 99999\r\n"[..]);

        bytes.reserve(50);
        write_content_length(590947, &mut bytes);
        assert_eq!(
            bytes.split().freeze(),
            b"\r\ncontent-length: 590947\r\n"[..]
        );
        bytes.reserve(50);
        write_content_length(999999, &mut bytes);
        assert_eq!(
            bytes.split().freeze(),
            b"\r\ncontent-length: 999999\r\n"[..]
        );
        bytes.reserve(50);
        write_content_length(5909471, &mut bytes);
        assert_eq!(
            bytes.split().freeze(),
            b"\r\ncontent-length: 5909471\r\n"[..]
        );
        bytes.reserve(50);
        write_content_length(59094718, &mut bytes);
        assert_eq!(
            bytes.split().freeze(),
            b"\r\ncontent-length: 59094718\r\n"[..]
        );
        bytes.reserve(50);
        write_content_length(4294973728, &mut bytes);
        assert_eq!(
            bytes.split().freeze(),
            b"\r\ncontent-length: 4294973728\r\n"[..]
        );
    }
}

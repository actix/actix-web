#![allow(unused_imports, unused_variables, dead_code)]
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::marker::PhantomData;
use std::str::FromStr;
use std::{cmp, fmt, io, mem};

use bytes::{BufMut, Bytes, BytesMut};
use http::header::{
    HeaderValue, ACCEPT_ENCODING, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING,
};
use http::{HeaderMap, Method, StatusCode, Version};

use crate::body::BodyLength;
use crate::config::ServiceConfig;
use crate::header::ContentEncoding;
use crate::helpers;
use crate::message::{ConnectionType, RequestHead, ResponseHead};
use crate::request::Request;
use crate::response::Response;

const AVERAGE_HEADER_SIZE: usize = 30;

#[derive(Debug)]
pub(crate) struct MessageEncoder<T: MessageType> {
    pub length: BodyLength,
    pub te: TransferEncoding,
    _t: PhantomData<T>,
}

impl<T: MessageType> Default for MessageEncoder<T> {
    fn default() -> Self {
        MessageEncoder {
            length: BodyLength::None,
            te: TransferEncoding::empty(),
            _t: PhantomData,
        }
    }
}

pub(crate) trait MessageType: Sized {
    fn status(&self) -> Option<StatusCode>;

    fn connection_type(&self) -> Option<ConnectionType>;

    fn headers(&self) -> &HeaderMap;

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()>;

    fn encode_headers(
        &mut self,
        dst: &mut BytesMut,
        version: Version,
        mut length: BodyLength,
        ctype: ConnectionType,
        config: &ServiceConfig,
    ) -> io::Result<()> {
        let mut skip_len = length != BodyLength::Stream;

        // Content length
        if let Some(status) = self.status() {
            match status {
                StatusCode::NO_CONTENT
                | StatusCode::CONTINUE
                | StatusCode::PROCESSING => length = BodyLength::None,
                StatusCode::SWITCHING_PROTOCOLS => {
                    skip_len = true;
                    length = BodyLength::Stream;
                }
                _ => (),
            }
        }
        match length {
            BodyLength::Chunked => {
                dst.extend_from_slice(b"\r\ntransfer-encoding: chunked\r\n")
            }
            BodyLength::Empty => {
                dst.extend_from_slice(b"\r\ncontent-length: 0\r\n");
            }
            BodyLength::Sized(len) => helpers::write_content_length(len, dst),
            BodyLength::Sized64(len) => {
                dst.extend_from_slice(b"\r\ncontent-length: ");
                write!(dst.writer(), "{}", len)?;
                dst.extend_from_slice(b"\r\n");
            }
            BodyLength::None | BodyLength::Stream => dst.extend_from_slice(b"\r\n"),
        }

        // Connection
        match ctype {
            ConnectionType::Upgrade => dst.extend_from_slice(b"connection: upgrade\r\n"),
            ConnectionType::KeepAlive if version < Version::HTTP_11 => {
                dst.extend_from_slice(b"connection: keep-alive\r\n")
            }
            ConnectionType::Close if version >= Version::HTTP_11 => {
                dst.extend_from_slice(b"connection: close\r\n")
            }
            _ => (),
        }

        // write headers
        let mut pos = 0;
        let mut has_date = false;
        let mut remaining = dst.remaining_mut();
        let mut buf = unsafe { &mut *(dst.bytes_mut() as *mut [u8]) };
        for (key, value) in self.headers() {
            match key {
                &CONNECTION => continue,
                &TRANSFER_ENCODING | &CONTENT_LENGTH if skip_len => continue,
                &DATE => {
                    has_date = true;
                }
                _ => (),
            }

            let v = value.as_ref();
            let k = key.as_str().as_bytes();
            let len = k.len() + v.len() + 4;
            if len > remaining {
                unsafe {
                    dst.advance_mut(pos);
                }
                pos = 0;
                dst.reserve(len);
                remaining = dst.remaining_mut();
                unsafe {
                    buf = &mut *(dst.bytes_mut() as *mut _);
                }
            }

            buf[pos..pos + k.len()].copy_from_slice(k);
            pos += k.len();
            buf[pos..pos + 2].copy_from_slice(b": ");
            pos += 2;
            buf[pos..pos + v.len()].copy_from_slice(v);
            pos += v.len();
            buf[pos..pos + 2].copy_from_slice(b"\r\n");
            pos += 2;
            remaining -= len;
        }
        unsafe {
            dst.advance_mut(pos);
        }

        // optimized date header, set_date writes \r\n
        if !has_date {
            config.set_date(dst);
        } else {
            // msg eof
            dst.extend_from_slice(b"\r\n");
        }

        Ok(())
    }
}

impl MessageType for Response<()> {
    fn status(&self) -> Option<StatusCode> {
        Some(self.head().status)
    }

    fn connection_type(&self) -> Option<ConnectionType> {
        self.head().ctype
    }

    fn headers(&self) -> &HeaderMap {
        &self.head().headers
    }

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()> {
        let head = self.head();
        let reason = head.reason().as_bytes();
        dst.reserve(256 + head.headers.len() * AVERAGE_HEADER_SIZE + reason.len());

        // status line
        helpers::write_status_line(head.version, head.status.as_u16(), dst);
        dst.extend_from_slice(reason);
        Ok(())
    }
}

impl MessageType for RequestHead {
    fn status(&self) -> Option<StatusCode> {
        None
    }

    fn connection_type(&self) -> Option<ConnectionType> {
        self.ctype
    }

    fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()> {
        write!(
            Writer(dst),
            "{} {} {}",
            self.method,
            self.uri.path_and_query().map(|u| u.as_str()).unwrap_or("/"),
            match self.version {
                Version::HTTP_09 => "HTTP/0.9",
                Version::HTTP_10 => "HTTP/1.0",
                Version::HTTP_11 => "HTTP/1.1",
                Version::HTTP_2 => "HTTP/2.0",
            }
        )
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

impl<T: MessageType> MessageEncoder<T> {
    /// Encode message
    pub fn encode_chunk(&mut self, msg: &[u8], buf: &mut BytesMut) -> io::Result<bool> {
        self.te.encode(msg, buf)
    }

    /// Encode eof
    pub fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        self.te.encode_eof(buf)
    }

    pub fn encode(
        &mut self,
        dst: &mut BytesMut,
        message: &mut T,
        head: bool,
        version: Version,
        length: BodyLength,
        ctype: ConnectionType,
        config: &ServiceConfig,
    ) -> io::Result<()> {
        // transfer encoding
        if !head {
            self.te = match length {
                BodyLength::Empty => TransferEncoding::empty(),
                BodyLength::Sized(len) => TransferEncoding::length(len as u64),
                BodyLength::Sized64(len) => TransferEncoding::length(len),
                BodyLength::Chunked => TransferEncoding::chunked(),
                BodyLength::Stream => TransferEncoding::eof(),
                BodyLength::None => TransferEncoding::empty(),
            };
        } else {
            self.te = TransferEncoding::empty();
        }

        message.encode_status(dst)?;
        message.encode_headers(dst, version, length, ctype, config)
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug)]
pub(crate) struct TransferEncoding {
    kind: TransferEncodingKind,
}

#[derive(Debug, PartialEq, Clone)]
enum TransferEncodingKind {
    /// An Encoder for when Transfer-Encoding includes `chunked`.
    Chunked(bool),
    /// An Encoder for when Content-Length is set.
    ///
    /// Enforces that the body is not longer than the Content-Length header.
    Length(u64),
    /// An Encoder for when Content-Length is not known.
    ///
    /// Application decides when to stop writing.
    Eof,
}

impl TransferEncoding {
    #[inline]
    pub fn empty() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(0),
        }
    }

    #[inline]
    pub fn eof() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline]
    pub fn chunked() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Chunked(false),
        }
    }

    #[inline]
    pub fn length(len: u64) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(len),
        }
    }

    /// Encode message. Return `EOF` state of encoder
    #[inline]
    pub fn encode(&mut self, msg: &[u8], buf: &mut BytesMut) -> io::Result<bool> {
        match self.kind {
            TransferEncodingKind::Eof => {
                let eof = msg.is_empty();
                buf.extend_from_slice(msg);
                Ok(eof)
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return Ok(true);
                }

                if msg.is_empty() {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                } else {
                    writeln!(Writer(buf), "{:X}\r", msg.len())
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

                    buf.reserve(msg.len() + 2);
                    buf.extend_from_slice(msg);
                    buf.extend_from_slice(b"\r\n");
                }
                Ok(*eof)
            }
            TransferEncodingKind::Length(ref mut remaining) => {
                if *remaining > 0 {
                    if msg.is_empty() {
                        return Ok(*remaining == 0);
                    }
                    let len = cmp::min(*remaining, msg.len() as u64);

                    buf.extend_from_slice(&msg[..len as usize]);

                    *remaining -= len as u64;
                    Ok(*remaining == 0)
                } else {
                    Ok(true)
                }
            }
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    #[inline]
    pub fn encode_eof(&mut self, buf: &mut BytesMut) -> io::Result<()> {
        match self.kind {
            TransferEncodingKind::Eof => Ok(()),
            TransferEncodingKind::Length(rem) => {
                if rem != 0 {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, ""))
                } else {
                    Ok(())
                }
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    buf.extend_from_slice(b"0\r\n\r\n");
                }
                Ok(())
            }
        }
    }
}

struct Writer<'a>(pub &'a mut BytesMut);

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
    use bytes::Bytes;

    #[test]
    fn test_chunked_te() {
        let mut bytes = BytesMut::new();
        let mut enc = TransferEncoding::chunked();
        {
            assert!(!enc.encode(b"test", &mut bytes).ok().unwrap());
            assert!(enc.encode(b"", &mut bytes).ok().unwrap());
        }
        assert_eq!(
            bytes.take().freeze(),
            Bytes::from_static(b"4\r\ntest\r\n0\r\n\r\n")
        );
    }
}

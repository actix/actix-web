#![allow(unused_imports, unused_variables, dead_code)]
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::marker::PhantomData;
use std::str::FromStr;
use std::{cmp, fmt, io, mem};

use bytes::{BufMut, Bytes, BytesMut};

use crate::body::BodySize;
use crate::config::ServiceConfig;
use crate::header::{map, ContentEncoding};
use crate::helpers;
use crate::http::header::{
    HeaderValue, ACCEPT_ENCODING, CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING,
};
use crate::http::{HeaderMap, Method, StatusCode, Version};
use crate::message::{ConnectionType, Head, RequestHead, ResponseHead};
use crate::request::Request;
use crate::response::Response;

const AVERAGE_HEADER_SIZE: usize = 30;

#[derive(Debug)]
pub(crate) struct MessageEncoder<T: MessageType> {
    pub length: BodySize,
    pub te: TransferEncoding,
    _t: PhantomData<T>,
}

impl<T: MessageType> Default for MessageEncoder<T> {
    fn default() -> Self {
        MessageEncoder {
            length: BodySize::None,
            te: TransferEncoding::empty(),
            _t: PhantomData,
        }
    }
}

pub(crate) trait MessageType: Sized {
    fn status(&self) -> Option<StatusCode>;

    fn headers(&self) -> &HeaderMap;

    fn camel_case(&self) -> bool {
        false
    }

    fn chunked(&self) -> bool;

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()>;

    fn encode_headers(
        &mut self,
        dst: &mut BytesMut,
        version: Version,
        mut length: BodySize,
        ctype: ConnectionType,
        config: &ServiceConfig,
    ) -> io::Result<()> {
        let chunked = self.chunked();
        let mut skip_len = length != BodySize::Stream;
        let camel_case = self.camel_case();

        // Content length
        if let Some(status) = self.status() {
            match status {
                StatusCode::NO_CONTENT
                | StatusCode::CONTINUE
                | StatusCode::PROCESSING => length = BodySize::None,
                StatusCode::SWITCHING_PROTOCOLS => {
                    skip_len = true;
                    length = BodySize::Stream;
                }
                _ => (),
            }
        }
        match length {
            BodySize::Stream => {
                if chunked {
                    if camel_case {
                        dst.put_slice(b"\r\nTransfer-Encoding: chunked\r\n")
                    } else {
                        dst.put_slice(b"\r\ntransfer-encoding: chunked\r\n")
                    }
                } else {
                    skip_len = false;
                    dst.put_slice(b"\r\n");
                }
            }
            BodySize::Empty => {
                if camel_case {
                    dst.put_slice(b"\r\nContent-Length: 0\r\n");
                } else {
                    dst.put_slice(b"\r\ncontent-length: 0\r\n");
                }
            }
            BodySize::Sized(len) => helpers::write_content_length(len, dst),
            BodySize::Sized64(len) => {
                if camel_case {
                    dst.put_slice(b"\r\nContent-Length: ");
                } else {
                    dst.put_slice(b"\r\ncontent-length: ");
                }
                write!(dst.writer(), "{}\r\n", len)?;
            }
            BodySize::None => dst.put_slice(b"\r\n"),
        }

        // Connection
        match ctype {
            ConnectionType::Upgrade => dst.put_slice(b"connection: upgrade\r\n"),
            ConnectionType::KeepAlive if version < Version::HTTP_11 => {
                if camel_case {
                    dst.put_slice(b"Connection: keep-alive\r\n")
                } else {
                    dst.put_slice(b"connection: keep-alive\r\n")
                }
            }
            ConnectionType::Close if version >= Version::HTTP_11 => {
                if camel_case {
                    dst.put_slice(b"Connection: close\r\n")
                } else {
                    dst.put_slice(b"connection: close\r\n")
                }
            }
            _ => (),
        }

        // write headers
        let mut pos = 0;
        let mut has_date = false;
        let mut remaining = dst.remaining_mut();
        let mut buf = unsafe { &mut *(dst.bytes_mut() as *mut [u8]) };
        for (key, value) in self.headers().inner.iter() {
            match *key {
                CONNECTION => continue,
                TRANSFER_ENCODING | CONTENT_LENGTH if skip_len => continue,
                DATE => {
                    has_date = true;
                }
                _ => (),
            }
            let k = key.as_str().as_bytes();
            match value {
                map::Value::One(ref val) => {
                    let v = val.as_ref();
                    let len = k.len() + v.len() + 4;
                    if len > remaining {
                        unsafe {
                            dst.advance_mut(pos);
                        }
                        pos = 0;
                        dst.reserve(len * 2);
                        remaining = dst.remaining_mut();
                        unsafe {
                            buf = &mut *(dst.bytes_mut() as *mut _);
                        }
                    }
                    // use upper Camel-Case
                    if camel_case {
                        write_camel_case(k, &mut buf[pos..pos + k.len()]);
                    } else {
                        buf[pos..pos + k.len()].copy_from_slice(k);
                    }
                    pos += k.len();
                    buf[pos..pos + 2].copy_from_slice(b": ");
                    pos += 2;
                    buf[pos..pos + v.len()].copy_from_slice(v);
                    pos += v.len();
                    buf[pos..pos + 2].copy_from_slice(b"\r\n");
                    pos += 2;
                    remaining -= len;
                }
                map::Value::Multi(ref vec) => {
                    for val in vec {
                        let v = val.as_ref();
                        let len = k.len() + v.len() + 4;
                        if len > remaining {
                            unsafe {
                                dst.advance_mut(pos);
                            }
                            pos = 0;
                            dst.reserve(len * 2);
                            remaining = dst.remaining_mut();
                            unsafe {
                                buf = &mut *(dst.bytes_mut() as *mut _);
                            }
                        }
                        // use upper Camel-Case
                        if camel_case {
                            write_camel_case(k, &mut buf[pos..pos + k.len()]);
                        } else {
                            buf[pos..pos + k.len()].copy_from_slice(k);
                        }
                        pos += k.len();
                        buf[pos..pos + 2].copy_from_slice(b": ");
                        pos += 2;
                        buf[pos..pos + v.len()].copy_from_slice(v);
                        pos += v.len();
                        buf[pos..pos + 2].copy_from_slice(b"\r\n");
                        pos += 2;
                        remaining -= len;
                    }
                }
            }
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

    fn chunked(&self) -> bool {
        self.head().chunked()
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
        dst.put_slice(reason);
        Ok(())
    }
}

impl MessageType for RequestHead {
    fn status(&self) -> Option<StatusCode> {
        None
    }

    fn chunked(&self) -> bool {
        self.chunked()
    }

    fn camel_case(&self) -> bool {
        RequestHead::camel_case_headers(self)
    }

    fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()> {
        dst.reserve(256 + self.headers.len() * AVERAGE_HEADER_SIZE);
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
        stream: bool,
        version: Version,
        length: BodySize,
        ctype: ConnectionType,
        config: &ServiceConfig,
    ) -> io::Result<()> {
        // transfer encoding
        if !head {
            self.te = match length {
                BodySize::Empty => TransferEncoding::empty(),
                BodySize::Sized(len) => TransferEncoding::length(len as u64),
                BodySize::Sized64(len) => TransferEncoding::length(len),
                BodySize::Stream => {
                    if message.chunked() && !stream {
                        TransferEncoding::chunked()
                    } else {
                        TransferEncoding::eof()
                    }
                }
                BodySize::None => TransferEncoding::empty(),
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

fn write_camel_case(value: &[u8], buffer: &mut [u8]) {
    let mut index = 0;
    let key = value;
    let mut key_iter = key.iter();

    if let Some(c) = key_iter.next() {
        if *c >= b'a' && *c <= b'z' {
            buffer[index] = *c ^ b' ';
            index += 1;
        }
    } else {
        return;
    }

    while let Some(c) = key_iter.next() {
        buffer[index] = *c;
        index += 1;
        if *c == b'-' {
            if let Some(c) = key_iter.next() {
                if *c >= b'a' && *c <= b'z' {
                    buffer[index] = *c ^ b' ';
                    index += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::http::header::{HeaderValue, CONTENT_TYPE};

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

    #[test]
    fn test_camel_case() {
        let mut bytes = BytesMut::with_capacity(2048);
        let mut head = RequestHead::default();
        head.set_camel_case_headers(true);
        head.headers.insert(DATE, HeaderValue::from_static("date"));
        head.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("plain/text"));

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Empty,
            ConnectionType::Close,
            &ServiceConfig::default(),
        );
        assert_eq!(
            bytes.take().freeze(),
            Bytes::from_static(b"\r\nContent-Length: 0\r\nConnection: close\r\nDate: date\r\nContent-Type: plain/text\r\n\r\n")
        );

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::KeepAlive,
            &ServiceConfig::default(),
        );
        assert_eq!(
            bytes.take().freeze(),
            Bytes::from_static(b"\r\nTransfer-Encoding: chunked\r\nDate: date\r\nContent-Type: plain/text\r\n\r\n")
        );

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Sized64(100),
            ConnectionType::KeepAlive,
            &ServiceConfig::default(),
        );
        assert_eq!(
            bytes.take().freeze(),
            Bytes::from_static(b"\r\nContent-Length: 100\r\nDate: date\r\nContent-Type: plain/text\r\n\r\n")
        );

        head.headers
            .append(CONTENT_TYPE, HeaderValue::from_static("xml"));
        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::KeepAlive,
            &ServiceConfig::default(),
        );
        assert_eq!(
            bytes.take().freeze(),
            Bytes::from_static(b"\r\nTransfer-Encoding: chunked\r\nDate: date\r\nContent-Type: xml\r\nContent-Type: plain/text\r\n\r\n")
        );

        head.set_camel_case_headers(false);
        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::KeepAlive,
            &ServiceConfig::default(),
        );
        assert_eq!(
            bytes.take().freeze(),
            Bytes::from_static(b"\r\ntransfer-encoding: chunked\r\ndate: date\r\ncontent-type: xml\r\ncontent-type: plain/text\r\n\r\n")
        );
    }
}

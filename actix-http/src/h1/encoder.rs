use std::io::Write;
use std::marker::PhantomData;
use std::ptr::copy_nonoverlapping;
use std::slice::from_raw_parts_mut;
use std::{cmp, io};

use bytes::{BufMut, BytesMut};

use crate::body::BodySize;
use crate::config::ServiceConfig;
use crate::header::map;
use crate::helpers;
use crate::http::header::{CONNECTION, CONTENT_LENGTH, DATE, TRANSFER_ENCODING};
use crate::http::{HeaderMap, StatusCode, Version};
use crate::message::{ConnectionType, RequestHeadType};
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

    fn extra_headers(&self) -> Option<&HeaderMap>;

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
                StatusCode::CONTINUE
                | StatusCode::SWITCHING_PROTOCOLS
                | StatusCode::PROCESSING
                | StatusCode::NO_CONTENT => {
                    // skip content-length and transfer-encoding headers
                    // See https://tools.ietf.org/html/rfc7230#section-3.3.1
                    // and https://tools.ietf.org/html/rfc7230#section-3.3.2
                    skip_len = true;
                    length = BodySize::None
                }
                _ => {}
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

        // merging headers from head and extra headers. HeaderMap::new() does not allocate.
        let empty_headers = HeaderMap::new();
        let extra_headers = self.extra_headers().unwrap_or(&empty_headers);
        let headers = self
            .headers()
            .inner
            .iter()
            .filter(|(name, _)| !extra_headers.contains_key(*name))
            .chain(extra_headers.inner.iter());

        // write headers

        let mut has_date = false;

        let mut buf = dst.bytes_mut().as_mut_ptr() as *mut u8;
        let mut remaining = dst.capacity() - dst.len();

        // tracks bytes written since last buffer resize
        // since buf is a raw pointer to a bytes container storage but is written to without the
        // container's knowledge, this is used to sync the containers cursor after data is written
        let mut pos = 0;

        for (key, value) in headers {
            match *key {
                CONNECTION => continue,
                TRANSFER_ENCODING | CONTENT_LENGTH if skip_len => continue,
                DATE => has_date = true,
                _ => (),
            }

            let k = key.as_str().as_bytes();
            let k_len = k.len();

            match value {
                map::Value::One(ref val) => {
                    let v = val.as_ref();
                    let v_len = v.len();

                    // key length + value length + colon + space + \r\n
                    let len = k_len + v_len + 4;

                    if len > remaining {
                        // not enough room in buffer for this header; reserve more space

                        // SAFETY: all the bytes written up to position "pos" are initialized
                        // the written byte count and pointer advancement are kept in sync
                        unsafe {
                            dst.advance_mut(pos);
                        }

                        pos = 0;
                        dst.reserve(len * 2);
                        remaining = dst.capacity() - dst.len();

                        // re-assign buf raw pointer since it's possible that the buffer was
                        // reallocated and/or resized
                        buf = dst.bytes_mut().as_mut_ptr() as *mut u8;
                    }

                    // SAFETY: on each write, it is enough to ensure that the advancement of the
                    // cursor matches the number of bytes written
                    unsafe {
                        // use upper Camel-Case
                        if camel_case {
                            write_camel_case(k, from_raw_parts_mut(buf, k_len))
                        } else {
                            write_data(k, buf, k_len)
                        }

                        buf = buf.add(k_len);

                        write_data(b": ", buf, 2);
                        buf = buf.add(2);

                        write_data(v, buf, v_len);
                        buf = buf.add(v_len);

                        write_data(b"\r\n", buf, 2);
                        buf = buf.add(2);
                    }

                    pos += len;
                    remaining -= len;
                }

                map::Value::Multi(ref vec) => {
                    for val in vec {
                        let v = val.as_ref();
                        let v_len = v.len();
                        let len = k_len + v_len + 4;

                        if len > remaining {
                            // SAFETY: all the bytes written up to position "pos" are initialized
                            // the written byte count and pointer advancement are kept in sync
                            unsafe {
                                dst.advance_mut(pos);
                            }
                            pos = 0;
                            dst.reserve(len * 2);
                            remaining = dst.capacity() - dst.len();

                            // re-assign buf raw pointer since it's possible that the buffer was
                            // reallocated and/or resized
                            buf = dst.bytes_mut().as_mut_ptr() as *mut u8;
                        }

                        // SAFETY: on each write, it is enough to ensure that the advancement of
                        // the cursor matches the number of bytes written
                        unsafe {
                            if camel_case {
                                write_camel_case(k, from_raw_parts_mut(buf, k_len));
                            } else {
                                write_data(k, buf, k_len);
                            }

                            buf = buf.add(k_len);

                            write_data(b": ", buf, 2);
                            buf = buf.add(2);

                            write_data(v, buf, v_len);
                            buf = buf.add(v_len);

                            write_data(b"\r\n", buf, 2);
                            buf = buf.add(2);
                        };

                        pos += len;
                        remaining -= len;
                    }
                }
            }
        }

        // final cursor synchronization with the bytes container
        //
        // SAFETY: all the bytes written up to position "pos" are initialized
        // the written byte count and pointer advancement are kept in sync
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

    fn extra_headers(&self) -> Option<&HeaderMap> {
        None
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

impl MessageType for RequestHeadType {
    fn status(&self) -> Option<StatusCode> {
        None
    }

    fn chunked(&self) -> bool {
        self.as_ref().chunked()
    }

    fn camel_case(&self) -> bool {
        self.as_ref().camel_case_headers()
    }

    fn headers(&self) -> &HeaderMap {
        self.as_ref().headers()
    }

    fn extra_headers(&self) -> Option<&HeaderMap> {
        self.extra_headers()
    }

    fn encode_status(&mut self, dst: &mut BytesMut) -> io::Result<()> {
        let head = self.as_ref();
        dst.reserve(256 + head.headers.len() * AVERAGE_HEADER_SIZE);
        write!(
            Writer(dst),
            "{} {} {}",
            head.method,
            head.uri.path_and_query().map(|u| u.as_str()).unwrap_or("/"),
            match head.version {
                Version::HTTP_09 => "HTTP/0.9",
                Version::HTTP_10 => "HTTP/1.0",
                Version::HTTP_11 => "HTTP/1.1",
                Version::HTTP_2 => "HTTP/2.0",
                Version::HTTP_3 => "HTTP/3.0",
                _ =>
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "unsupported version"
                    )),
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
                BodySize::Sized(len) => TransferEncoding::length(len),
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

/// # Safety
/// Callers must ensure that the given length matches given value length.
unsafe fn write_data(value: &[u8], buf: *mut u8, len: usize) {
    debug_assert_eq!(value.len(), len);
    copy_nonoverlapping(value.as_ptr(), buf, len);
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
    use std::rc::Rc;

    use bytes::Bytes;
    use http::header::AUTHORIZATION;

    use super::*;
    use crate::http::header::{HeaderValue, CONTENT_TYPE};
    use crate::RequestHead;

    #[test]
    fn test_chunked_te() {
        let mut bytes = BytesMut::new();
        let mut enc = TransferEncoding::chunked();
        {
            assert!(!enc.encode(b"test", &mut bytes).ok().unwrap());
            assert!(enc.encode(b"", &mut bytes).ok().unwrap());
        }
        assert_eq!(
            bytes.split().freeze(),
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

        let mut head = RequestHeadType::Owned(head);

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Empty,
            ConnectionType::Close,
            &ServiceConfig::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("Content-Length: 0\r\n"));
        assert!(data.contains("Connection: close\r\n"));
        assert!(data.contains("Content-Type: plain/text\r\n"));
        assert!(data.contains("Date: date\r\n"));

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::KeepAlive,
            &ServiceConfig::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("Transfer-Encoding: chunked\r\n"));
        assert!(data.contains("Content-Type: plain/text\r\n"));
        assert!(data.contains("Date: date\r\n"));

        let mut head = RequestHead::default();
        head.set_camel_case_headers(false);
        head.headers.insert(DATE, HeaderValue::from_static("date"));
        head.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("plain/text"));
        head.headers
            .append(CONTENT_TYPE, HeaderValue::from_static("xml"));

        let mut head = RequestHeadType::Owned(head);
        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::KeepAlive,
            &ServiceConfig::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("transfer-encoding: chunked\r\n"));
        assert!(data.contains("content-type: xml\r\n"));
        assert!(data.contains("content-type: plain/text\r\n"));
        assert!(data.contains("date: date\r\n"));
    }

    #[test]
    fn test_extra_headers() {
        let mut bytes = BytesMut::with_capacity(2048);

        let mut head = RequestHead::default();
        head.headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("some authorization"),
        );

        let mut extra_headers = HeaderMap::new();
        extra_headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("another authorization"),
        );
        extra_headers.insert(DATE, HeaderValue::from_static("date"));

        let mut head = RequestHeadType::Rc(Rc::new(head), Some(extra_headers));

        let _ = head.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Empty,
            ConnectionType::Close,
            &ServiceConfig::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(data.contains("content-length: 0\r\n"));
        assert!(data.contains("connection: close\r\n"));
        assert!(data.contains("authorization: another authorization\r\n"));
        assert!(data.contains("date: date\r\n"));
    }

    #[test]
    fn test_no_content_length() {
        let mut bytes = BytesMut::with_capacity(2048);

        let mut res: Response<()> =
            Response::new(StatusCode::SWITCHING_PROTOCOLS).into_body::<()>();
        res.headers_mut()
            .insert(DATE, HeaderValue::from_static(&""));
        res.headers_mut()
            .insert(CONTENT_LENGTH, HeaderValue::from_static(&"0"));

        let _ = res.encode_headers(
            &mut bytes,
            Version::HTTP_11,
            BodySize::Stream,
            ConnectionType::Upgrade,
            &ServiceConfig::default(),
        );
        let data =
            String::from_utf8(Vec::from(bytes.split().freeze().as_ref())).unwrap();
        assert!(!data.contains("content-length: 0\r\n"));
        assert!(!data.contains("transfer-encoding: chunked\r\n"));
    }
}

use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::str::FromStr;
use std::{cmp, fmt, io, mem};

#[cfg(feature = "brotli")]
use brotli2::write::BrotliEncoder;
use bytes::BytesMut;
#[cfg(feature = "flate2")]
use flate2::write::{GzEncoder, ZlibEncoder};
#[cfg(feature = "flate2")]
use flate2::Compression;
use http::header::{ACCEPT_ENCODING, CONTENT_LENGTH};
use http::{StatusCode, Version};

use super::message::InnerRequest;
use body::{Binary, Body};
use header::ContentEncoding;
use httpresponse::HttpResponse;

#[derive(Debug)]
pub(crate) enum ResponseLength {
    Chunked,
    Zero,
    Length(usize),
    Length64(u64),
    None,
}

#[derive(Debug)]
pub(crate) struct ResponseInfo {
    head: bool,
    pub length: ResponseLength,
    pub content_encoding: Option<&'static str>,
}

impl ResponseInfo {
    pub fn new(head: bool) -> Self {
        ResponseInfo {
            head,
            length: ResponseLength::None,
            content_encoding: None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Output {
    Empty(BytesMut),
    Buffer(BytesMut),
    Encoder(ContentEncoder),
    TE(TransferEncoding),
    Done,
}

impl Output {
    pub fn take(&mut self) -> BytesMut {
        match mem::replace(self, Output::Done) {
            Output::Empty(bytes) => bytes,
            Output::Buffer(bytes) => bytes,
            Output::Encoder(mut enc) => enc.take_buf(),
            Output::TE(mut te) => te.take(),
            Output::Done => panic!(),
        }
    }

    pub fn take_option(&mut self) -> Option<BytesMut> {
        match mem::replace(self, Output::Done) {
            Output::Empty(bytes) => Some(bytes),
            Output::Buffer(bytes) => Some(bytes),
            Output::Encoder(mut enc) => Some(enc.take_buf()),
            Output::TE(mut te) => Some(te.take()),
            Output::Done => None,
        }
    }

    pub fn as_ref(&mut self) -> &BytesMut {
        match self {
            Output::Empty(ref mut bytes) => bytes,
            Output::Buffer(ref mut bytes) => bytes,
            Output::Encoder(ref mut enc) => enc.buf_ref(),
            Output::TE(ref mut te) => te.buf_ref(),
            Output::Done => panic!(),
        }
    }
    pub fn as_mut(&mut self) -> &mut BytesMut {
        match self {
            Output::Empty(ref mut bytes) => bytes,
            Output::Buffer(ref mut bytes) => bytes,
            Output::Encoder(ref mut enc) => enc.buf_mut(),
            Output::TE(ref mut te) => te.buf_mut(),
            Output::Done => panic!(),
        }
    }
    pub fn split_to(&mut self, cap: usize) -> BytesMut {
        match self {
            Output::Empty(ref mut bytes) => bytes.split_to(cap),
            Output::Buffer(ref mut bytes) => bytes.split_to(cap),
            Output::Encoder(ref mut enc) => enc.buf_mut().split_to(cap),
            Output::TE(ref mut te) => te.buf_mut().split_to(cap),
            Output::Done => BytesMut::new(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Output::Empty(ref bytes) => bytes.len(),
            Output::Buffer(ref bytes) => bytes.len(),
            Output::Encoder(ref enc) => enc.len(),
            Output::TE(ref te) => te.len(),
            Output::Done => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Output::Empty(ref bytes) => bytes.is_empty(),
            Output::Buffer(ref bytes) => bytes.is_empty(),
            Output::Encoder(ref enc) => enc.is_empty(),
            Output::TE(ref te) => te.is_empty(),
            Output::Done => true,
        }
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match self {
            Output::Buffer(ref mut bytes) => {
                bytes.extend_from_slice(data);
                Ok(())
            }
            Output::Encoder(ref mut enc) => enc.write(data),
            Output::TE(ref mut te) => te.encode(data).map(|_| ()),
            Output::Empty(_) | Output::Done => Ok(()),
        }
    }

    pub fn write_eof(&mut self) -> Result<bool, io::Error> {
        match self {
            Output::Buffer(_) => Ok(true),
            Output::Encoder(ref mut enc) => enc.write_eof(),
            Output::TE(ref mut te) => Ok(te.encode_eof()),
            Output::Empty(_) | Output::Done => Ok(true),
        }
    }

    pub(crate) fn for_server(
        &mut self, info: &mut ResponseInfo, req: &InnerRequest, resp: &mut HttpResponse,
        response_encoding: ContentEncoding,
    ) {
        let buf = self.take();
        let version = resp.version().unwrap_or_else(|| req.version);
        let mut len = 0;

        let has_body = match resp.body() {
            Body::Empty => false,
            Body::Binary(ref bin) => {
                len = bin.len();
                !(response_encoding == ContentEncoding::Auto && len < 96)
            }
            _ => true,
        };

        // Enable content encoding only if response does not contain Content-Encoding
        // header
        #[cfg(any(feature = "brotli", feature = "flate2"))]
        let mut encoding = if has_body {
            let encoding = match response_encoding {
                ContentEncoding::Auto => {
                    // negotiate content-encoding
                    if let Some(val) = req.headers.get(ACCEPT_ENCODING) {
                        if let Ok(enc) = val.to_str() {
                            AcceptEncoding::parse(enc)
                        } else {
                            ContentEncoding::Identity
                        }
                    } else {
                        ContentEncoding::Identity
                    }
                }
                encoding => encoding,
            };
            if encoding.is_compression() {
                info.content_encoding = Some(encoding.as_str());
            }
            encoding
        } else {
            ContentEncoding::Identity
        };
        #[cfg(not(any(feature = "brotli", feature = "flate2")))]
        let mut encoding = ContentEncoding::Identity;

        let transfer = match resp.body() {
            Body::Empty => {
                info.length = match resp.status() {
                    StatusCode::NO_CONTENT
                    | StatusCode::CONTINUE
                    | StatusCode::SWITCHING_PROTOCOLS
                    | StatusCode::PROCESSING => ResponseLength::None,
                    _ => ResponseLength::Zero,
                };
                *self = Output::Empty(buf);
                return;
            }
            Body::Binary(_) => {
                #[cfg(any(feature = "brotli", feature = "flate2"))]
                {
                    if !(encoding == ContentEncoding::Identity
                        || encoding == ContentEncoding::Auto)
                    {
                        let mut tmp = BytesMut::new();
                        let mut transfer = TransferEncoding::eof(tmp);
                        let mut enc = match encoding {
                            #[cfg(feature = "flate2")]
                            ContentEncoding::Deflate => ContentEncoder::Deflate(
                                ZlibEncoder::new(transfer, Compression::fast()),
                            ),
                            #[cfg(feature = "flate2")]
                            ContentEncoding::Gzip => ContentEncoder::Gzip(
                                GzEncoder::new(transfer, Compression::fast()),
                            ),
                            #[cfg(feature = "brotli")]
                            ContentEncoding::Br => {
                                ContentEncoder::Br(BrotliEncoder::new(transfer, 3))
                            }
                            ContentEncoding::Identity | ContentEncoding::Auto => {
                                unreachable!()
                            }
                        };

                        let bin = resp.replace_body(Body::Empty).binary();

                        // TODO return error!
                        let _ = enc.write(bin.as_ref());
                        let _ = enc.write_eof();
                        let body = enc.buf_mut().take();
                        len = body.len();
                        resp.replace_body(Binary::from(body));
                    }
                }

                info.length = ResponseLength::Length(len);
                if info.head {
                    *self = Output::Empty(buf);
                } else {
                    *self = Output::Buffer(buf);
                }
                return;
            }
            Body::Streaming(_) | Body::Actor(_) => {
                if resp.upgrade() {
                    if version == Version::HTTP_2 {
                        error!("Connection upgrade is forbidden for HTTP/2");
                    }
                    if encoding != ContentEncoding::Identity {
                        encoding = ContentEncoding::Identity;
                        info.content_encoding.take();
                    }
                    TransferEncoding::eof(buf)
                } else {
                    if !(encoding == ContentEncoding::Identity
                        || encoding == ContentEncoding::Auto)
                    {
                        resp.headers_mut().remove(CONTENT_LENGTH);
                    }
                    Output::streaming_encoding(info, buf, version, resp)
                }
            }
        };
        // check for head response
        if info.head {
            resp.set_body(Body::Empty);
            *self = Output::Empty(transfer.buf.unwrap());
            return;
        }

        let enc = match encoding {
            #[cfg(feature = "flate2")]
            ContentEncoding::Deflate => {
                ContentEncoder::Deflate(ZlibEncoder::new(transfer, Compression::fast()))
            }
            #[cfg(feature = "flate2")]
            ContentEncoding::Gzip => {
                ContentEncoder::Gzip(GzEncoder::new(transfer, Compression::fast()))
            }
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => ContentEncoder::Br(BrotliEncoder::new(transfer, 3)),
            ContentEncoding::Identity | ContentEncoding::Auto => {
                *self = Output::TE(transfer);
                return;
            }
        };
        *self = Output::Encoder(enc);
    }

    fn streaming_encoding(
        info: &mut ResponseInfo, buf: BytesMut, version: Version,
        resp: &mut HttpResponse,
    ) -> TransferEncoding {
        match resp.chunked() {
            Some(true) => {
                // Enable transfer encoding
                info.length = ResponseLength::Chunked;
                if version == Version::HTTP_11 {
                    TransferEncoding::chunked(buf)
                } else {
                    TransferEncoding::eof(buf)
                }
            }
            Some(false) => TransferEncoding::eof(buf),
            None => {
                // if Content-Length is specified, then use it as length hint
                let (len, chunked) =
                    if let Some(len) = resp.headers().get(CONTENT_LENGTH) {
                        // Content-Length
                        if let Ok(s) = len.to_str() {
                            if let Ok(len) = s.parse::<u64>() {
                                (Some(len), false)
                            } else {
                                error!("illegal Content-Length: {:?}", len);
                                (None, false)
                            }
                        } else {
                            error!("illegal Content-Length: {:?}", len);
                            (None, false)
                        }
                    } else {
                        (None, true)
                    };

                if !chunked {
                    if let Some(len) = len {
                        info.length = ResponseLength::Length64(len);
                        TransferEncoding::length(len, buf)
                    } else {
                        TransferEncoding::eof(buf)
                    }
                } else {
                    // Enable transfer encoding
                    info.length = ResponseLength::Chunked;
                    if version == Version::HTTP_11 {
                        TransferEncoding::chunked(buf)
                    } else {
                        TransferEncoding::eof(buf)
                    }
                }
            }
        }
    }
}

pub(crate) enum ContentEncoder {
    #[cfg(feature = "flate2")]
    Deflate(ZlibEncoder<TransferEncoding>),
    #[cfg(feature = "flate2")]
    Gzip(GzEncoder<TransferEncoding>),
    #[cfg(feature = "brotli")]
    Br(BrotliEncoder<TransferEncoding>),
    Identity(TransferEncoding),
}

impl fmt::Debug for ContentEncoder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(_) => writeln!(f, "ContentEncoder(Brotli)"),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(_) => writeln!(f, "ContentEncoder(Deflate)"),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(_) => writeln!(f, "ContentEncoder(Gzip)"),
            ContentEncoder::Identity(_) => writeln!(f, "ContentEncoder(Identity)"),
        }
    }
}

impl ContentEncoder {
    #[inline]
    pub fn len(&self) -> usize {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref encoder) => encoder.get_ref().len(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref encoder) => encoder.get_ref().len(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref encoder) => encoder.get_ref().len(),
            ContentEncoder::Identity(ref encoder) => encoder.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref encoder) => encoder.get_ref().is_empty(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref encoder) => encoder.get_ref().is_empty(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref encoder) => encoder.get_ref().is_empty(),
            ContentEncoder::Identity(ref encoder) => encoder.is_empty(),
        }
    }

    #[inline]
    pub(crate) fn take_buf(&mut self) -> BytesMut {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => encoder.get_mut().take(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().take(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().take(),
            ContentEncoder::Identity(ref mut encoder) => encoder.take(),
        }
    }

    #[inline]
    pub(crate) fn buf_mut(&mut self) -> &mut BytesMut {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => encoder.get_mut().buf_mut(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().buf_mut(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().buf_mut(),
            ContentEncoder::Identity(ref mut encoder) => encoder.buf_mut(),
        }
    }

    #[inline]
    pub(crate) fn buf_ref(&mut self) -> &BytesMut {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => encoder.get_mut().buf_ref(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().buf_ref(),
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().buf_ref(),
            ContentEncoder::Identity(ref mut encoder) => encoder.buf_ref(),
        }
    }

    #[cfg_attr(feature = "cargo-clippy", allow(clippy::inline_always))]
    #[inline(always)]
    pub fn write_eof(&mut self) -> Result<bool, io::Error> {
        let encoder =
            mem::replace(self, ContentEncoder::Identity(TransferEncoding::empty()));

        match encoder {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(encoder) => match encoder.finish() {
                Ok(mut writer) => {
                    writer.encode_eof();
                    *self = ContentEncoder::Identity(writer);
                    Ok(true)
                }
                Err(err) => Err(err),
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(encoder) => match encoder.finish() {
                Ok(mut writer) => {
                    writer.encode_eof();
                    *self = ContentEncoder::Identity(writer);
                    Ok(true)
                }
                Err(err) => Err(err),
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(encoder) => match encoder.finish() {
                Ok(mut writer) => {
                    writer.encode_eof();
                    *self = ContentEncoder::Identity(writer);
                    Ok(true)
                }
                Err(err) => Err(err),
            },
            ContentEncoder::Identity(mut writer) => {
                let res = writer.encode_eof();
                *self = ContentEncoder::Identity(writer);
                Ok(res)
            }
        }
    }

    #[cfg_attr(feature = "cargo-clippy", allow(clippy::inline_always))]
    #[inline(always)]
    pub fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding br encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Gzip(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding gzip encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(feature = "flate2")]
            ContentEncoder::Deflate(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding deflate encoding: {}", err);
                    Err(err)
                }
            },
            ContentEncoder::Identity(ref mut encoder) => {
                encoder.encode(data)?;
                Ok(())
            }
        }
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug)]
pub(crate) struct TransferEncoding {
    buf: Option<BytesMut>,
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
    fn take(&mut self) -> BytesMut {
        self.buf.take().unwrap()
    }

    fn buf_ref(&mut self) -> &BytesMut {
        self.buf.as_ref().unwrap()
    }

    fn len(&self) -> usize {
        self.buf.as_ref().unwrap().len()
    }

    fn is_empty(&self) -> bool {
        self.buf.as_ref().unwrap().is_empty()
    }

    fn buf_mut(&mut self) -> &mut BytesMut {
        self.buf.as_mut().unwrap()
    }

    #[inline]
    pub fn empty() -> TransferEncoding {
        TransferEncoding {
            buf: None,
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline]
    pub fn eof(buf: BytesMut) -> TransferEncoding {
        TransferEncoding {
            buf: Some(buf),
            kind: TransferEncodingKind::Eof,
        }
    }

    #[inline]
    pub fn chunked(buf: BytesMut) -> TransferEncoding {
        TransferEncoding {
            buf: Some(buf),
            kind: TransferEncodingKind::Chunked(false),
        }
    }

    #[inline]
    pub fn length(len: u64, buf: BytesMut) -> TransferEncoding {
        TransferEncoding {
            buf: Some(buf),
            kind: TransferEncodingKind::Length(len),
        }
    }

    /// Encode message. Return `EOF` state of encoder
    #[inline]
    pub fn encode(&mut self, msg: &[u8]) -> io::Result<bool> {
        match self.kind {
            TransferEncodingKind::Eof => {
                let eof = msg.is_empty();
                self.buf.as_mut().unwrap().extend_from_slice(msg);
                Ok(eof)
            }
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return Ok(true);
                }

                if msg.is_empty() {
                    *eof = true;
                    self.buf.as_mut().unwrap().extend_from_slice(b"0\r\n\r\n");
                } else {
                    let mut buf = BytesMut::new();
                    writeln!(&mut buf, "{:X}\r", msg.len())
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

                    let b = self.buf.as_mut().unwrap();
                    b.reserve(buf.len() + msg.len() + 2);
                    b.extend_from_slice(buf.as_ref());
                    b.extend_from_slice(msg);
                    b.extend_from_slice(b"\r\n");
                }
                Ok(*eof)
            }
            TransferEncodingKind::Length(ref mut remaining) => {
                if *remaining > 0 {
                    if msg.is_empty() {
                        return Ok(*remaining == 0);
                    }
                    let len = cmp::min(*remaining, msg.len() as u64);

                    self.buf
                        .as_mut()
                        .unwrap()
                        .extend_from_slice(&msg[..len as usize]);

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
    pub fn encode_eof(&mut self) -> bool {
        match self.kind {
            TransferEncodingKind::Eof => true,
            TransferEncodingKind::Length(rem) => rem == 0,
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    self.buf.as_mut().unwrap().extend_from_slice(b"0\r\n\r\n");
                }
                true
            }
        }
    }
}

impl io::Write for TransferEncoding {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.buf.is_some() {
            self.encode(buf)?;
        }
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct AcceptEncoding {
    encoding: ContentEncoding,
    quality: f64,
}

impl Eq for AcceptEncoding {}

impl Ord for AcceptEncoding {
    fn cmp(&self, other: &AcceptEncoding) -> cmp::Ordering {
        if self.quality > other.quality {
            cmp::Ordering::Less
        } else if self.quality < other.quality {
            cmp::Ordering::Greater
        } else {
            cmp::Ordering::Equal
        }
    }
}

impl PartialOrd for AcceptEncoding {
    fn partial_cmp(&self, other: &AcceptEncoding) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AcceptEncoding {
    fn eq(&self, other: &AcceptEncoding) -> bool {
        self.quality == other.quality
    }
}

impl AcceptEncoding {
    fn new(tag: &str) -> Option<AcceptEncoding> {
        let parts: Vec<&str> = tag.split(';').collect();
        let encoding = match parts.len() {
            0 => return None,
            _ => ContentEncoding::from(parts[0]),
        };
        let quality = match parts.len() {
            1 => encoding.quality(),
            _ => match f64::from_str(parts[1]) {
                Ok(q) => q,
                Err(_) => 0.0,
            },
        };
        Some(AcceptEncoding { encoding, quality })
    }

    /// Parse a raw Accept-Encoding header value into an ordered list.
    pub fn parse(raw: &str) -> ContentEncoding {
        let mut encodings: Vec<_> = raw
            .replace(' ', "")
            .split(',')
            .map(|l| AcceptEncoding::new(l))
            .collect();
        encodings.sort();

        for enc in encodings {
            if let Some(enc) = enc {
                return enc.encoding;
            }
        }
        ContentEncoding::Identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_chunked_te() {
        let bytes = BytesMut::new();
        let mut enc = TransferEncoding::chunked(bytes);
        {
            assert!(!enc.encode(b"test").ok().unwrap());
            assert!(enc.encode(b"").ok().unwrap());
        }
        assert_eq!(
            enc.buf_mut().take().freeze(),
            Bytes::from_static(b"4\r\ntest\r\n0\r\n\r\n")
        );
    }
}

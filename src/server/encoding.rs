use std::{io, cmp, mem};
use std::io::{Read, Write};
use std::fmt::Write as FmtWrite;
use std::str::FromStr;

use http::Version;
use http::header::{HeaderMap, HeaderValue,
                   ACCEPT_ENCODING, CONNECTION,
                   CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::{GzEncoder, DeflateDecoder, DeflateEncoder};
use brotli2::write::{BrotliDecoder, BrotliEncoder};
use bytes::{Bytes, BytesMut, BufMut, Writer};

use headers::ContentEncoding;
use body::{Body, Binary};
use error::PayloadError;
use helpers::SharedBytes;
use httprequest::HttpMessage;
use httpresponse::HttpResponse;
use payload::{PayloadSender, PayloadWriter};

impl ContentEncoding {

    #[inline]
    fn is_compression(&self) -> bool {
        match *self {
            ContentEncoding::Identity | ContentEncoding::Auto => false,
            _ => true
        }
    }

    fn as_str(&self) -> &'static str {
        match *self {
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }
    /// default quality value
    fn quality(&self) -> f64 {
        match *self {
            ContentEncoding::Br => 1.1,
            ContentEncoding::Gzip => 1.0,
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::Identity | ContentEncoding::Auto => 0.1,
        }
    }
}

// TODO: remove memory allocation
impl<'a> From<&'a str> for ContentEncoding {
    fn from(s: &'a str) -> ContentEncoding {
        match s.trim().to_lowercase().as_ref() {
            "br" => ContentEncoding::Br,
            "gzip" => ContentEncoding::Gzip,
            "deflate" => ContentEncoding::Deflate,
            "identity" => ContentEncoding::Identity,
            _ => ContentEncoding::Auto,
        }
    }
}


pub(crate) enum PayloadType {
    Sender(PayloadSender),
    Encoding(Box<EncodedPayload>),
}

impl PayloadType {

    pub fn new(headers: &HeaderMap, sender: PayloadSender) -> PayloadType {
        // check content-encoding
        let enc = if let Some(enc) = headers.get(CONTENT_ENCODING) {
            if let Ok(enc) = enc.to_str() {
                ContentEncoding::from(enc)
            } else {
                ContentEncoding::Auto
            }
        } else {
            ContentEncoding::Auto
        };

        match enc {
            ContentEncoding::Auto | ContentEncoding::Identity =>
                PayloadType::Sender(sender),
            _ => PayloadType::Encoding(Box::new(EncodedPayload::new(sender, enc))),
        }
    }
}

impl PayloadWriter for PayloadType {
    fn set_error(&mut self, err: PayloadError) {
        match *self {
            PayloadType::Sender(ref mut sender) => sender.set_error(err),
            PayloadType::Encoding(ref mut enc) => enc.set_error(err),
        }
    }

    fn feed_eof(&mut self) {
        match *self {
            PayloadType::Sender(ref mut sender) => sender.feed_eof(),
            PayloadType::Encoding(ref mut enc) => enc.feed_eof(),
        }
    }

    fn feed_data(&mut self, data: Bytes) {
        match *self {
            PayloadType::Sender(ref mut sender) => sender.feed_data(data),
            PayloadType::Encoding(ref mut enc) => enc.feed_data(data),
        }
    }

    fn capacity(&self) -> usize {
        match *self {
            PayloadType::Sender(ref sender) => sender.capacity(),
            PayloadType::Encoding(ref enc) => enc.capacity(),
        }
    }
}

enum Decoder {
    Deflate(Box<DeflateDecoder<Writer<BytesMut>>>),
    Gzip(Option<Box<GzDecoder<Wrapper>>>),
    Br(Box<BrotliDecoder<Writer<BytesMut>>>),
    Identity,
}

// should go after write::GzDecoder get implemented
#[derive(Debug)]
struct Wrapper {
    buf: BytesMut,
    eof: bool,
}

impl io::Read for Wrapper {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = cmp::min(buf.len(), self.buf.len());
        buf[..len].copy_from_slice(&self.buf[..len]);
        self.buf.split_to(len);
        if len == 0 {
            if self.eof {
                Ok(0)
            } else {
                Err(io::Error::new(io::ErrorKind::WouldBlock, ""))
            }
        } else {
            Ok(len)
        }
    }
}

impl io::Write for Wrapper {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Payload wrapper with content decompression support
pub(crate) struct EncodedPayload {
    inner: PayloadSender,
    decoder: Decoder,
    dst: BytesMut,
    error: bool,
}

impl EncodedPayload {
    pub fn new(inner: PayloadSender, enc: ContentEncoding) -> EncodedPayload {
        let dec = match enc {
            ContentEncoding::Br => Decoder::Br(
                Box::new(BrotliDecoder::new(BytesMut::with_capacity(8192).writer()))),
            ContentEncoding::Deflate => Decoder::Deflate(
                Box::new(DeflateDecoder::new(BytesMut::with_capacity(8192).writer()))),
            ContentEncoding::Gzip => Decoder::Gzip(None),
            _ => Decoder::Identity,
        };
        EncodedPayload{ inner: inner, decoder: dec, error: false, dst: BytesMut::new() }
    }
}

impl PayloadWriter for EncodedPayload {

    fn set_error(&mut self, err: PayloadError) {
        self.inner.set_error(err)
    }

    fn feed_eof(&mut self) {
        if self.error {
            return
        }
        let err = match self.decoder {
            Decoder::Br(ref mut decoder) => {
                match decoder.finish() {
                    Ok(mut writer) => {
                        let b = writer.get_mut().take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        self.inner.feed_eof();
                        return
                    },
                    Err(err) => Some(err),
                }
            },
            Decoder::Gzip(ref mut decoder) => {
                if let Some(ref mut decoder) = *decoder {
                    decoder.as_mut().get_mut().eof = true;

                    loop {
                        self.dst.reserve(8192);
                        match decoder.read(unsafe{self.dst.bytes_mut()}) {
                            Ok(n) =>  {
                                if n == 0 {
                                    self.inner.feed_eof();
                                    return
                                } else {
                                    unsafe{self.dst.set_len(n)};
                                    self.inner.feed_data(self.dst.split_to(n).freeze());
                                }
                            }
                            Err(err) => {
                                break Some(err);
                            }
                        }
                    }
                } else {
                    return
                }
            },
            Decoder::Deflate(ref mut decoder) => {
                match decoder.try_finish() {
                    Ok(_) => {
                        let b = decoder.get_mut().get_mut().take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        self.inner.feed_eof();
                        return
                    },
                    Err(err) => Some(err),
                }
            },
            Decoder::Identity => {
                self.inner.feed_eof();
                return
            }
        };

        self.error = true;
        self.decoder = Decoder::Identity;
        if let Some(err) = err {
            self.set_error(PayloadError::ParseError(err));
        } else {
            self.set_error(PayloadError::Incomplete);
        }
    }

    fn feed_data(&mut self, data: Bytes) {
        if self.error {
            return
        }
        match self.decoder {
            Decoder::Br(ref mut decoder) => {
                if decoder.write(&data).is_ok() && decoder.flush().is_ok() {
                    let b = decoder.get_mut().get_mut().take().freeze();
                    if !b.is_empty() {
                        self.inner.feed_data(b);
                    }
                    return
                }
                trace!("Error decoding br encoding");
            }

            Decoder::Gzip(ref mut decoder) => {
                if decoder.is_none() {
                    *decoder = Some(
                        Box::new(GzDecoder::new(
                            Wrapper{buf: BytesMut::from(data), eof: false})));
                } else {
                    let _ = decoder.as_mut().unwrap().write(&data);
                }

                loop {
                    self.dst.reserve(8192);
                    match decoder.as_mut().as_mut().unwrap().read(unsafe{self.dst.bytes_mut()}) {
                        Ok(n) =>  {
                            if n == 0 {
                                return
                            } else {
                                unsafe{self.dst.set_len(n)};
                                self.inner.feed_data(self.dst.split_to(n).freeze());
                            }
                        }
                        Err(e) => {
                            if e.kind() == io::ErrorKind::WouldBlock {
                                return
                            }
                            break
                        }
                    }
                }
            }

            Decoder::Deflate(ref mut decoder) => {
                if decoder.write(&data).is_ok() && decoder.flush().is_ok() {
                    let b = decoder.get_mut().get_mut().take().freeze();
                    if !b.is_empty() {
                        self.inner.feed_data(b);
                    }
                    return
                }
                trace!("Error decoding deflate encoding");
            }
            Decoder::Identity => {
                self.inner.feed_data(data);
                return
            }
        };

        self.error = true;
        self.decoder = Decoder::Identity;
        self.set_error(PayloadError::EncodingCorrupted);
    }

    fn capacity(&self) -> usize {
        self.inner.capacity()
    }
}

pub(crate) struct PayloadEncoder(ContentEncoder);

impl PayloadEncoder {

    pub fn empty(bytes: SharedBytes) -> PayloadEncoder {
        PayloadEncoder(ContentEncoder::Identity(TransferEncoding::eof(bytes)))
    }

    pub fn new(buf: SharedBytes, req: &HttpMessage, resp: &mut HttpResponse) -> PayloadEncoder {
        let version = resp.version().unwrap_or_else(|| req.version);
        let mut body = resp.replace_body(Body::Empty);
        let has_body = match body {
            Body::Empty => false,
            Body::Binary(ref bin) => bin.len() >= 512,
            _ => true,
        };

        // Enable content encoding only if response does not contain Content-Encoding header
        let mut encoding = if has_body {
            let encoding = match *resp.content_encoding() {
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
                resp.headers_mut().insert(
                    CONTENT_ENCODING, HeaderValue::from_static(encoding.as_str()));
            }
            encoding
        } else {
            ContentEncoding::Identity
        };

        let transfer = match body {
            Body::Empty => {
                resp.headers_mut().remove(CONTENT_LENGTH);
                TransferEncoding::eof(buf)
            },
            Body::Binary(ref mut bytes) => {
                if encoding.is_compression() {
                    let tmp = SharedBytes::default();
                    let transfer = TransferEncoding::eof(tmp.clone());
                    let mut enc = match encoding {
                        ContentEncoding::Deflate => ContentEncoder::Deflate(
                            DeflateEncoder::new(transfer, Compression::default())),
                        ContentEncoding::Gzip => ContentEncoder::Gzip(
                            GzEncoder::new(transfer, Compression::default())),
                        ContentEncoding::Br => ContentEncoder::Br(
                            BrotliEncoder::new(transfer, 5)),
                        ContentEncoding::Identity => ContentEncoder::Identity(transfer),
                        ContentEncoding::Auto => unreachable!()
                    };
                    // TODO return error!
                    let _ = enc.write(bytes.as_ref());
                    let _ = enc.write_eof();

                    *bytes = Binary::from(tmp.get_mut().take());
                    encoding = ContentEncoding::Identity;
                }
                resp.headers_mut().remove(CONTENT_LENGTH);
                TransferEncoding::eof(buf)
            }
            Body::Streaming(_) | Body::Actor(_) => {
                if resp.upgrade() {
                    if version == Version::HTTP_2 {
                        error!("Connection upgrade is forbidden for HTTP/2");
                    } else {
                        resp.headers_mut().insert(
                            CONNECTION, HeaderValue::from_static("upgrade"));
                    }
                    if encoding != ContentEncoding::Identity {
                        encoding = ContentEncoding::Identity;
                        resp.headers_mut().remove(CONTENT_ENCODING);
                    }
                    TransferEncoding::eof(buf)
                } else {
                    PayloadEncoder::streaming_encoding(buf, version, resp)
                }
            }
        };
        resp.replace_body(body);

        PayloadEncoder(
            match encoding {
                ContentEncoding::Deflate => ContentEncoder::Deflate(
                    DeflateEncoder::new(transfer, Compression::default())),
                ContentEncoding::Gzip => ContentEncoder::Gzip(
                    GzEncoder::new(transfer, Compression::default())),
                ContentEncoding::Br => ContentEncoder::Br(
                    BrotliEncoder::new(transfer, 5)),
                ContentEncoding::Identity => ContentEncoder::Identity(transfer),
                ContentEncoding::Auto => unreachable!()
            }
        )
    }

    fn streaming_encoding(buf: SharedBytes, version: Version,
                          resp: &mut HttpResponse) -> TransferEncoding {
        match resp.chunked() {
            Some(true) => {
                // Enable transfer encoding
                resp.headers_mut().remove(CONTENT_LENGTH);
                if version == Version::HTTP_2 {
                    resp.headers_mut().remove(TRANSFER_ENCODING);
                    TransferEncoding::eof(buf)
                } else {
                    resp.headers_mut().insert(
                        TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                    TransferEncoding::chunked(buf)
                }
            },
            Some(false) =>
                TransferEncoding::eof(buf),
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
                        TransferEncoding::length(len, buf)
                    } else {
                        TransferEncoding::eof(buf)
                    }
                } else {
                    // Enable transfer encoding
                    match version {
                        Version::HTTP_11 => {
                            resp.headers_mut().insert(
                                TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                            TransferEncoding::chunked(buf)
                        },
                        _ => {
                            resp.headers_mut().remove(TRANSFER_ENCODING);
                            TransferEncoding::eof(buf)
                        }
                    }
                }
            }
        }
    }
}

impl PayloadEncoder {

    #[inline]
    pub fn len(&self) -> usize {
        self.0.get_ref().len()
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut BytesMut {
        self.0.get_mut()
    }

    #[inline]
    pub fn is_eof(&self) -> bool {
        self.0.is_eof()
    }

    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    #[inline(always)]
    pub fn write(&mut self, payload: &[u8]) -> Result<(), io::Error> {
        self.0.write(payload)
    }

    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    #[inline(always)]
    pub fn write_eof(&mut self) -> Result<(), io::Error> {
        self.0.write_eof()
    }
}

enum ContentEncoder {
    Deflate(DeflateEncoder<TransferEncoding>),
    Gzip(GzEncoder<TransferEncoding>),
    Br(BrotliEncoder<TransferEncoding>),
    Identity(TransferEncoding),
}

impl ContentEncoder {

    #[inline]
    pub fn is_eof(&self) -> bool {
        match *self {
            ContentEncoder::Br(ref encoder) =>
                encoder.get_ref().is_eof(),
            ContentEncoder::Deflate(ref encoder) =>
                encoder.get_ref().is_eof(),
            ContentEncoder::Gzip(ref encoder) =>
                encoder.get_ref().is_eof(),
            ContentEncoder::Identity(ref encoder) =>
                encoder.is_eof(),
        }
    }

    #[inline]
    pub fn get_ref(&self) -> &BytesMut {
        match *self {
            ContentEncoder::Br(ref encoder) =>
                encoder.get_ref().buffer.get_ref(),
            ContentEncoder::Deflate(ref encoder) =>
                encoder.get_ref().buffer.get_ref(),
            ContentEncoder::Gzip(ref encoder) =>
                encoder.get_ref().buffer.get_ref(),
            ContentEncoder::Identity(ref encoder) =>
                encoder.buffer.get_ref(),
        }
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut BytesMut {
        match *self {
            ContentEncoder::Br(ref mut encoder) =>
                encoder.get_mut().buffer.get_mut(),
            ContentEncoder::Deflate(ref mut encoder) =>
                encoder.get_mut().buffer.get_mut(),
            ContentEncoder::Gzip(ref mut encoder) =>
                encoder.get_mut().buffer.get_mut(),
            ContentEncoder::Identity(ref mut encoder) =>
                encoder.buffer.get_mut(),
        }
    }

    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    #[inline(always)]
    pub fn write_eof(&mut self) -> Result<(), io::Error> {
        let encoder = mem::replace(
            self, ContentEncoder::Identity(TransferEncoding::eof(SharedBytes::empty())));

        match encoder {
            ContentEncoder::Br(encoder) => {
                match encoder.finish() {
                    Ok(mut writer) => {
                        writer.encode_eof();
                        *self = ContentEncoder::Identity(writer);
                        Ok(())
                    },
                    Err(err) => Err(err),
                }
            }
            ContentEncoder::Gzip(encoder) => {
                match encoder.finish() {
                    Ok(mut writer) => {
                        writer.encode_eof();
                        *self = ContentEncoder::Identity(writer);
                        Ok(())
                    },
                    Err(err) => Err(err),
                }
            },
            ContentEncoder::Deflate(encoder) => {
                match encoder.finish() {
                    Ok(mut writer) => {
                        writer.encode_eof();
                        *self = ContentEncoder::Identity(writer);
                        Ok(())
                    },
                    Err(err) => Err(err),
                }
            },
            ContentEncoder::Identity(mut writer) => {
                writer.encode_eof();
                *self = ContentEncoder::Identity(writer);
                Ok(())
            }
        }
    }

    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    #[inline(always)]
    pub fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match *self {
            ContentEncoder::Br(ref mut encoder) => {
                match encoder.write(data) {
                    Ok(_) =>
                        encoder.flush(),
                    Err(err) => {
                        trace!("Error decoding br encoding: {}", err);
                        Err(err)
                    },
                }
            },
            ContentEncoder::Gzip(ref mut encoder) => {
                match encoder.write(data) {
                    Ok(_) =>
                        encoder.flush(),
                    Err(err) => {
                        trace!("Error decoding gzip encoding: {}", err);
                        Err(err)
                    },
                }
            }
            ContentEncoder::Deflate(ref mut encoder) => {
                match encoder.write(data) {
                    Ok(_) =>
                        encoder.flush(),
                    Err(err) => {
                        trace!("Error decoding deflate encoding: {}", err);
                        Err(err)
                    },
                }
            }
            ContentEncoder::Identity(ref mut encoder) => {
                encoder.encode(data)?;
                Ok(())
            }
        }
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug, Clone)]
pub(crate) struct TransferEncoding {
    kind: TransferEncodingKind,
    buffer: SharedBytes,
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
    /// Appliction decides when to stop writing.
    Eof,
}

impl TransferEncoding {

    #[inline]
    pub fn eof(bytes: SharedBytes) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
            buffer: bytes,
        }
    }

    #[inline]
    pub fn chunked(bytes: SharedBytes) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Chunked(false),
            buffer: bytes,
        }
    }

    #[inline]
    pub fn length(len: u64, bytes: SharedBytes) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(len),
            buffer: bytes,
        }
    }

    #[inline]
    pub fn is_eof(&self) -> bool {
        match self.kind {
            TransferEncodingKind::Eof => true,
            TransferEncodingKind::Chunked(ref eof) => *eof,
            TransferEncodingKind::Length(ref remaining) => *remaining == 0,
        }
    }

    /// Encode message. Return `EOF` state of encoder
    #[inline]
    pub fn encode(&mut self, msg: &[u8]) -> io::Result<bool> {
        match self.kind {
            TransferEncodingKind::Eof => {
                self.buffer.get_mut().extend_from_slice(msg);
                Ok(msg.is_empty())
            },
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return Ok(true);
                }

                if msg.is_empty() {
                    *eof = true;
                    self.buffer.get_mut().extend_from_slice(b"0\r\n\r\n");
                } else {
                    write!(self.buffer.get_mut(), "{:X}\r\n", msg.len())
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                    self.buffer.get_mut().extend_from_slice(msg);
                    self.buffer.get_mut().extend_from_slice(b"\r\n");
                }
                Ok(*eof)
            },
            TransferEncodingKind::Length(ref mut remaining) => {
                if msg.is_empty() {
                    return Ok(*remaining == 0)
                }
                let max = cmp::min(*remaining, msg.len() as u64);
                self.buffer.get_mut().extend_from_slice(msg[..max as usize].as_ref());

                *remaining -= max as u64;
                Ok(*remaining == 0)
            },
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    #[inline]
    pub fn encode_eof(&mut self) {
        match self.kind {
            TransferEncodingKind::Eof | TransferEncodingKind::Length(_) => (),
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    self.buffer.get_mut().extend_from_slice(b"0\r\n\r\n");
                }
            },
        }
    }
}

impl io::Write for TransferEncoding {

    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.encode(buf)?;
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
            }
        };
        Some(AcceptEncoding {
            encoding: encoding,
            quality: quality,
        })
    }

    /// Parse a raw Accept-Encoding header value into an ordered list.
    pub fn parse(raw: &str) -> ContentEncoding {
        let mut encodings: Vec<_> =
            raw.replace(' ', "").split(',').map(|l| AcceptEncoding::new(l)).collect();
        encodings.sort();

        for enc in encodings {
            if let Some(enc) = enc {
                return enc.encoding
            }
        }
        ContentEncoding::Identity
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunked_te() {
        let bytes = SharedBytes::default();
        let mut enc = TransferEncoding::chunked(bytes.clone());
        assert!(!enc.encode(b"test").ok().unwrap());
        assert!(enc.encode(b"").ok().unwrap());
        assert_eq!(bytes.get_mut().take().freeze(),
                   Bytes::from_static(b"4\r\ntest\r\n0\r\n\r\n"));
    }
}

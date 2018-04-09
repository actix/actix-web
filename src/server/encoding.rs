use std::{io, cmp, mem};
use std::io::{Read, Write};
use std::fmt::Write as FmtWrite;
use std::str::FromStr;

use bytes::{Bytes, BytesMut, BufMut};
use http::{Version, Method, HttpTryFrom};
use http::header::{HeaderMap, HeaderValue,
                   ACCEPT_ENCODING, CONNECTION,
                   CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::{GzEncoder, DeflateDecoder, DeflateEncoder};
#[cfg(feature="brotli")]
use brotli2::write::{BrotliDecoder, BrotliEncoder};

use header::ContentEncoding;
use body::{Body, Binary};
use error::PayloadError;
use httprequest::HttpInnerMessage;
use httpresponse::HttpResponse;
use payload::{PayloadSender, PayloadWriter, PayloadStatus};

use super::shared::SharedBytes;

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
    #[inline]
    fn set_error(&mut self, err: PayloadError) {
        match *self {
            PayloadType::Sender(ref mut sender) => sender.set_error(err),
            PayloadType::Encoding(ref mut enc) => enc.set_error(err),
        }
    }

    #[inline]
    fn feed_eof(&mut self) {
        match *self {
            PayloadType::Sender(ref mut sender) => sender.feed_eof(),
            PayloadType::Encoding(ref mut enc) => enc.feed_eof(),
        }
    }

    #[inline]
    fn feed_data(&mut self, data: Bytes) {
        match *self {
            PayloadType::Sender(ref mut sender) => sender.feed_data(data),
            PayloadType::Encoding(ref mut enc) => enc.feed_data(data),
        }
    }

    #[inline]
    fn need_read(&self) -> PayloadStatus {
        match *self {
            PayloadType::Sender(ref sender) => sender.need_read(),
            PayloadType::Encoding(ref enc) => enc.need_read(),
        }
    }
}


/// Payload wrapper with content decompression support
pub(crate) struct EncodedPayload {
    inner: PayloadSender,
    error: bool,
    payload: PayloadStream,
}

impl EncodedPayload {
    pub fn new(inner: PayloadSender, enc: ContentEncoding) -> EncodedPayload {
        EncodedPayload{ inner, error: false, payload: PayloadStream::new(enc) }
    }
}

impl PayloadWriter for EncodedPayload {

    fn set_error(&mut self, err: PayloadError) {
        self.inner.set_error(err)
    }

    fn feed_eof(&mut self) {
        if !self.error {
            match self.payload.feed_eof() {
                Err(err) => {
                    self.error = true;
                    self.set_error(PayloadError::Io(err));
                },
                Ok(value) => {
                    if let Some(b) = value {
                        self.inner.feed_data(b);
                    }
                    self.inner.feed_eof();
                }
            }
        }
    }

    fn feed_data(&mut self, data: Bytes) {
        if self.error {
            return
        }

        match self.payload.feed_data(data) {
            Ok(Some(b)) => self.inner.feed_data(b),
            Ok(None) => (),
            Err(e) => {
                self.error = true;
                self.set_error(e.into());
            }
        }
    }

    #[inline]
    fn need_read(&self) -> PayloadStatus {
        self.inner.need_read()
    }
}

pub(crate) enum Decoder {
    Deflate(Box<DeflateDecoder<Writer>>),
    Gzip(Option<Box<GzDecoder<Wrapper>>>),
    #[cfg(feature="brotli")]
    Br(Box<BrotliDecoder<Writer>>),
    Identity,
}

// should go after write::GzDecoder get implemented
#[derive(Debug)]
pub(crate) struct Wrapper {
    pub buf: BytesMut,
    pub eof: bool,
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

pub(crate) struct Writer {
    buf: BytesMut,
}

impl Writer {
    fn new() -> Writer {
        Writer{buf: BytesMut::with_capacity(8192)}
    }
    fn take(&mut self) -> Bytes {
        self.buf.take().freeze()
    }
}

impl io::Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Payload stream with decompression support
pub(crate) struct PayloadStream {
    decoder: Decoder,
    dst: BytesMut,
}

impl PayloadStream {
    pub fn new(enc: ContentEncoding) -> PayloadStream {
        let dec = match enc {
            #[cfg(feature="brotli")]
            ContentEncoding::Br => Decoder::Br(
                Box::new(BrotliDecoder::new(Writer::new()))),
            ContentEncoding::Deflate => Decoder::Deflate(
                Box::new(DeflateDecoder::new(Writer::new()))),
            ContentEncoding::Gzip => Decoder::Gzip(None),
            _ => Decoder::Identity,
        };
        PayloadStream{ decoder: dec, dst: BytesMut::new() }
    }
}

impl PayloadStream {

    pub fn feed_eof(&mut self) -> io::Result<Option<Bytes>> {
        match self.decoder {
            #[cfg(feature="brotli")]
            Decoder::Br(ref mut decoder) => {
                match decoder.finish() {
                    Ok(mut writer) => {
                        let b = writer.take();
                        if !b.is_empty() {
                            Ok(Some(b))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(e) => Err(e),
                }
            },
            Decoder::Gzip(ref mut decoder) => {
                if let Some(ref mut decoder) = *decoder {
                    decoder.as_mut().get_mut().eof = true;

                    self.dst.reserve(8192);
                    match decoder.read(unsafe{self.dst.bytes_mut()}) {
                        Ok(n) =>  {
                            unsafe{self.dst.advance_mut(n)};
                            return Ok(Some(self.dst.take().freeze()))
                        }
                        Err(e) =>
                            return Err(e),
                    }
                } else {
                    Ok(None)
                }
            },
            Decoder::Deflate(ref mut decoder) => {
                match decoder.try_finish() {
                    Ok(_) => {
                        let b = decoder.get_mut().take();
                        if !b.is_empty() {
                            Ok(Some(b))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(e) => Err(e),
                }
            },
            Decoder::Identity => Ok(None),
        }
    }

    pub fn feed_data(&mut self, data: Bytes) -> io::Result<Option<Bytes>> {
        match self.decoder {
            #[cfg(feature="brotli")]
            Decoder::Br(ref mut decoder) => {
                match decoder.write_all(&data) {
                    Ok(_) => {
                        decoder.flush()?;
                        let b = decoder.get_mut().take();
                        if !b.is_empty() {
                            Ok(Some(b))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(e) => Err(e)
                }
            },
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
                    match decoder.as_mut()
                        .as_mut().unwrap().read(unsafe{self.dst.bytes_mut()})
                    {
                        Ok(n) =>  {
                            if n != 0 {
                                unsafe{self.dst.advance_mut(n)};
                            }
                            if n == 0 {
                                return Ok(Some(self.dst.take().freeze()));
                            }
                        }
                        Err(e) => {
                            if e.kind() == io::ErrorKind::WouldBlock && !self.dst.is_empty()
                            {
                                return Ok(Some(self.dst.take().freeze()));
                            }
                            return Err(e)
                        }
                    }
                }
            },
            Decoder::Deflate(ref mut decoder) => {
                match decoder.write_all(&data) {
                    Ok(_) => {
                        decoder.flush()?;
                        let b = decoder.get_mut().take();
                        if !b.is_empty() {
                            Ok(Some(b))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(e) => Err(e),
                }
            },
            Decoder::Identity => Ok(Some(data)),
        }
    }
}

pub(crate) enum ContentEncoder {
    Deflate(DeflateEncoder<TransferEncoding>),
    Gzip(GzEncoder<TransferEncoding>),
    #[cfg(feature="brotli")]
    Br(BrotliEncoder<TransferEncoding>),
    Identity(TransferEncoding),
}

impl ContentEncoder {

    pub fn empty(bytes: SharedBytes) -> ContentEncoder {
        ContentEncoder::Identity(TransferEncoding::eof(bytes))
    }

    pub fn for_server(buf: SharedBytes,
                      req: &HttpInnerMessage,
                      resp: &mut HttpResponse,
                      response_encoding: ContentEncoding) -> ContentEncoder
    {
        let version = resp.version().unwrap_or_else(|| req.version);
        let is_head = req.method == Method::HEAD;
        let mut body = resp.replace_body(Body::Empty);
        let has_body = match body {
            Body::Empty => false,
            Body::Binary(ref bin) =>
                !(response_encoding == ContentEncoding::Auto && bin.len() < 96),
            _ => true,
        };

        // Enable content encoding only if response does not contain Content-Encoding header
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
                resp.headers_mut().insert(
                    CONTENT_ENCODING, HeaderValue::from_static(encoding.as_str()));
            }
            encoding
        } else {
            ContentEncoding::Identity
        };

        let mut transfer = match body {
            Body::Empty => {
                if req.method != Method::HEAD {
                    resp.headers_mut().remove(CONTENT_LENGTH);
                }
                TransferEncoding::length(0, buf)
            },
            Body::Binary(ref mut bytes) => {
                if !(encoding == ContentEncoding::Identity
                     || encoding == ContentEncoding::Auto)
                {
                    let tmp = SharedBytes::default();
                    let transfer = TransferEncoding::eof(tmp.clone());
                    let mut enc = match encoding {
                        ContentEncoding::Deflate => ContentEncoder::Deflate(
                            DeflateEncoder::new(transfer, Compression::fast())),
                        ContentEncoding::Gzip => ContentEncoder::Gzip(
                            GzEncoder::new(transfer, Compression::fast())),
                        #[cfg(feature="brotli")]
                        ContentEncoding::Br => ContentEncoder::Br(
                            BrotliEncoder::new(transfer, 3)),
                        ContentEncoding::Identity => ContentEncoder::Identity(transfer),
                        ContentEncoding::Auto => unreachable!()
                    };
                    // TODO return error!
                    let _ = enc.write(bytes.clone());
                    let _ = enc.write_eof();

                    *bytes = Binary::from(tmp.take());
                    encoding = ContentEncoding::Identity;
                }
                if is_head {
                    let mut b = BytesMut::new();
                    let _ = write!(b, "{}", bytes.len());
                    resp.headers_mut().insert(
                        CONTENT_LENGTH, HeaderValue::try_from(b.freeze()).unwrap());
                } else {
                    // resp.headers_mut().remove(CONTENT_LENGTH);
                }
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
                    ContentEncoder::streaming_encoding(buf, version, resp)
                }
            }
        };
        //
        if is_head {
            transfer.kind = TransferEncodingKind::Length(0);
        } else {
            resp.replace_body(body);
        }

        match encoding {
            ContentEncoding::Deflate => ContentEncoder::Deflate(
                DeflateEncoder::new(transfer, Compression::fast())),
            ContentEncoding::Gzip => ContentEncoder::Gzip(
                GzEncoder::new(transfer, Compression::fast())),
            #[cfg(feature="brotli")]
            ContentEncoding::Br => ContentEncoder::Br(
                BrotliEncoder::new(transfer, 3)),
            ContentEncoding::Identity | ContentEncoding::Auto =>
                ContentEncoder::Identity(transfer),
        }
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

impl ContentEncoder {

    #[inline]
    pub fn is_eof(&self) -> bool {
        match *self {
            #[cfg(feature="brotli")]
            ContentEncoder::Br(ref encoder) => encoder.get_ref().is_eof(),
            ContentEncoder::Deflate(ref encoder) => encoder.get_ref().is_eof(),
            ContentEncoder::Gzip(ref encoder) => encoder.get_ref().is_eof(),
            ContentEncoder::Identity(ref encoder) => encoder.is_eof(),
        }
    }

    #[cfg_attr(feature = "cargo-clippy", allow(inline_always))]
    #[inline(always)]
    pub fn write_eof(&mut self) -> Result<(), io::Error> {
        let encoder = mem::replace(
            self, ContentEncoder::Identity(TransferEncoding::eof(SharedBytes::empty())));

        match encoder {
            #[cfg(feature="brotli")]
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
    pub fn write(&mut self, data: Binary) -> Result<(), io::Error> {
        match *self {
            #[cfg(feature="brotli")]
            ContentEncoder::Br(ref mut encoder) => {
                match encoder.write_all(data.as_ref()) {
                    Ok(_) => Ok(()),
                    Err(err) => {
                        trace!("Error decoding br encoding: {}", err);
                        Err(err)
                    },
                }
            },
            ContentEncoder::Gzip(ref mut encoder) => {
                match encoder.write_all(data.as_ref()) {
                    Ok(_) => Ok(()),
                    Err(err) => {
                        trace!("Error decoding gzip encoding: {}", err);
                        Err(err)
                    },
                }
            }
            ContentEncoder::Deflate(ref mut encoder) => {
                match encoder.write_all(data.as_ref()) {
                    Ok(_) => Ok(()),
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
    /// Application decides when to stop writing.
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
    pub fn encode(&mut self, mut msg: Binary) -> io::Result<bool> {
        match self.kind {
            TransferEncodingKind::Eof => {
                let eof = msg.is_empty();
                self.buffer.extend(msg);
                Ok(eof)
            },
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return Ok(true);
                }

                if msg.is_empty() {
                    *eof = true;
                    self.buffer.extend_from_slice(b"0\r\n\r\n");
                } else {
                    let mut buf = BytesMut::new();
                    writeln!(&mut buf, "{:X}\r", msg.len())
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                    self.buffer.reserve(buf.len() + msg.len() + 2);
                    self.buffer.extend(buf.into());
                    self.buffer.extend(msg);
                    self.buffer.extend_from_slice(b"\r\n");
                }
                Ok(*eof)
            },
            TransferEncodingKind::Length(ref mut remaining) => {
                if *remaining > 0 {
                    if msg.is_empty() {
                        return Ok(*remaining == 0)
                    }
                    let len = cmp::min(*remaining, msg.len() as u64);
                    self.buffer.extend(msg.take().split_to(len as usize).into());

                    *remaining -= len as u64;
                    Ok(*remaining == 0)
                } else {
                    Ok(true)
                }
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
                    self.buffer.extend_from_slice(b"0\r\n\r\n");
                }
            },
        }
    }
}

impl io::Write for TransferEncoding {

    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.encode(Binary::from_slice(buf))?;
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
        Some(AcceptEncoding{ encoding, quality })
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
        assert!(!enc.encode(Binary::from(b"test".as_ref())).ok().unwrap());
        assert!(enc.encode(Binary::from(b"".as_ref())).ok().unwrap());
        assert_eq!(bytes.get_mut().take().freeze(),
                   Bytes::from_static(b"4\r\ntest\r\n0\r\n\r\n"));
    }
}

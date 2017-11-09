use std::{io, cmp, mem};
use std::io::{Read, Write};
use std::fmt::Write as FmtWrite;
use std::str::FromStr;

use http::Version;
use http::header::{HeaderMap, HeaderValue,
                   ACCEPT_ENCODING, CONNECTION,
                   CONTENT_ENCODING, CONTENT_LENGTH, TRANSFER_ENCODING};
use flate2::Compression;
use flate2::read::{GzDecoder};
use flate2::write::{GzEncoder, DeflateDecoder, DeflateEncoder};
use brotli2::write::{BrotliDecoder, BrotliEncoder};
use bytes::{Bytes, BytesMut, BufMut, Writer};

use body::Body;
use httprequest::HttpRequest;
use httpresponse::HttpResponse;
use payload::{PayloadSender, PayloadWriter, PayloadError};

/// Represents supported types of content encodings
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ContentEncoding {
    /// Automatically select encoding based on encoding negotiation
    Auto,
    /// A format using the Brotli algorithm
    Br,
    /// A format using the zlib structure with deflate algorithm
    Deflate,
    /// Gzip algorithm
    Gzip,
    /// Indicates the identity function (i.e. no compression, nor modification)
    Identity,
}

impl ContentEncoding {
    fn as_str(&self) -> &'static str {
        match *self {
            ContentEncoding::Br => "br",
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Deflate => "deflate",
            ContentEncoding::Identity | ContentEncoding::Auto => "identity",
        }
    }
    // default quality
    fn quality(&self) -> f64 {
        match *self {
            ContentEncoding::Br => 1.1,
            ContentEncoding::Gzip => 1.0,
            ContentEncoding::Deflate => 0.9,
            ContentEncoding::Identity | ContentEncoding::Auto => 0.1,
        }
    }
}

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
    Encoding(EncodedPayload),
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
            _ => PayloadType::Encoding(EncodedPayload::new(sender, enc)),
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
    Deflate(DeflateDecoder<BytesWriter>),
    Gzip(Option<GzDecoder<Wrapper>>),
    Br(BrotliDecoder<BytesWriter>),
    Identity,
}

// should go after write::GzDecoder get implemented
#[derive(Debug)]
struct Wrapper {
    buf: BytesMut
}

impl io::Read for Wrapper {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = cmp::min(buf.len(), self.buf.len());
        buf[..len].copy_from_slice(&self.buf[..len]);
        self.buf.split_to(len);
        Ok(len)
    }
}

struct BytesWriter {
    buf: BytesMut,
}

impl Default for BytesWriter {
    fn default() -> BytesWriter {
        BytesWriter{buf: BytesMut::with_capacity(8192)}
    }
}

impl io::Write for BytesWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend(buf);
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
    dst: Writer<BytesMut>,
    error: bool,
}

impl EncodedPayload {
    pub fn new(inner: PayloadSender, enc: ContentEncoding) -> EncodedPayload {
        let dec = match enc {
            ContentEncoding::Br => Decoder::Br(
                BrotliDecoder::new(BytesWriter::default())),
            ContentEncoding::Deflate => Decoder::Deflate(
                DeflateDecoder::new(BytesWriter::default())),
            ContentEncoding::Gzip => Decoder::Gzip(None),
            _ => Decoder::Identity,
        };
        EncodedPayload {
            inner: inner,
            decoder: dec,
            error: false,
            dst: BytesMut::new().writer(),
        }
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
                        let b = writer.buf.take().freeze();
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
                if decoder.is_none() {
                    self.inner.feed_eof();
                    return
                }
                loop {
                    let len = self.dst.get_ref().len();
                    let len_buf = decoder.as_mut().unwrap().get_mut().buf.len();

                    if len < len_buf * 2 {
                        self.dst.get_mut().reserve(len_buf * 2 - len);
                        unsafe{self.dst.get_mut().set_len(len_buf * 2)};
                    }
                    match decoder.as_mut().unwrap().read(&mut self.dst.get_mut()) {
                        Ok(n) =>  {
                            if n == 0 {
                                self.inner.feed_eof();
                                return
                            } else {
                                self.inner.feed_data(self.dst.get_mut().split_to(n).freeze());
                            }
                        }
                        Err(err) => break Some(err)
                    }
                }
            },
            Decoder::Deflate(ref mut decoder) => {
                match decoder.try_finish() {
                    Ok(_) => {
                        let b = decoder.get_mut().buf.take().freeze();
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
                    let b = decoder.get_mut().buf.take().freeze();
                    if !b.is_empty() {
                        self.inner.feed_data(b);
                    }
                    return
                }
                trace!("Error decoding br encoding");
            }

            Decoder::Gzip(ref mut decoder) => {
                if decoder.is_none() {
                    let mut buf = BytesMut::new();
                    buf.extend(data);
                    *decoder = Some(GzDecoder::new(Wrapper{buf: buf}).unwrap());
                } else {
                    decoder.as_mut().unwrap().get_mut().buf.extend(data);
                }

                loop {
                    let len_buf = decoder.as_mut().unwrap().get_mut().buf.len();
                    if len_buf == 0 {
                        return
                    }

                    let len = self.dst.get_ref().len();
                    if len < len_buf * 2 {
                        self.dst.get_mut().reserve(len_buf * 2 - len);
                        unsafe{self.dst.get_mut().set_len(len_buf * 2)};
                    }
                    match decoder.as_mut().unwrap().read(&mut self.dst.get_mut()) {
                        Ok(n) =>  {
                            if n == 0 {
                                return
                            } else {
                                self.inner.feed_data(self.dst.get_mut().split_to(n).freeze());
                            }
                        }
                        Err(_) => break
                    }
                }
            }

            Decoder::Deflate(ref mut decoder) => {
                if decoder.write(&data).is_ok() && decoder.flush().is_ok() {
                    let b = decoder.get_mut().buf.take().freeze();
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

impl Default for PayloadEncoder {
    fn default() -> PayloadEncoder {
        PayloadEncoder(ContentEncoder::Identity(TransferEncoding::eof()))
    }
}

impl PayloadEncoder {

    pub fn new(req: &HttpRequest, resp: &mut HttpResponse) -> PayloadEncoder {
        let version = resp.version().unwrap_or_else(|| req.version());
        let body = resp.replace_body(Body::Empty);
        let has_body = if let Body::Empty = body { false } else { true };

        // Enable content encoding only if response does not contain Content-Encoding header
        let encoding = if has_body && !resp.headers.contains_key(CONTENT_ENCODING) {
            let encoding = match *resp.content_encoding() {
                ContentEncoding::Auto => {
                    // negotiate content-encoding
                    if let Some(val) = req.headers().get(ACCEPT_ENCODING) {
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
            resp.headers.insert(CONTENT_ENCODING, HeaderValue::from_static(encoding.as_str()));
            encoding
        } else {
            ContentEncoding::Identity
        };

        // in general case it is very expensive to get compressed payload length,
        // just switch to chunked encoding
        let compression = encoding != ContentEncoding::Identity;

        let transfer = match body {
            Body::Empty => {
                if resp.chunked() {
                    error!("Chunked transfer is enabled but body is set to Empty");
                }
                resp.headers.insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
                resp.headers.remove(TRANSFER_ENCODING);
                TransferEncoding::length(0)
            },
            Body::Length(n) => {
                if resp.chunked() {
                    error!("Chunked transfer is enabled but body with specific length is specified");
                }
                if compression {
                    resp.headers.remove(CONTENT_LENGTH);
                    if version == Version::HTTP_2 {
                        resp.headers.remove(TRANSFER_ENCODING);
                        TransferEncoding::eof()
                    } else {
                        resp.headers.insert(
                            TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                        TransferEncoding::chunked()
                    }
                } else {
                    resp.headers.insert(
                        CONTENT_LENGTH,
                        HeaderValue::from_str(format!("{}", n).as_str()).unwrap());
                    resp.headers.remove(TRANSFER_ENCODING);
                    TransferEncoding::length(n)
                }
            },
            Body::Binary(ref bytes) => {
                if compression {
                    resp.headers.remove(CONTENT_LENGTH);
                    if version == Version::HTTP_2 {
                        resp.headers.remove(TRANSFER_ENCODING);
                        TransferEncoding::eof()
                    } else {
                        resp.headers.insert(
                            TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                        TransferEncoding::chunked()
                    }
                } else {
                    resp.headers.insert(
                        CONTENT_LENGTH,
                        HeaderValue::from_str(format!("{}", bytes.len()).as_str()).unwrap());
                    resp.headers.remove(TRANSFER_ENCODING);
                    TransferEncoding::length(bytes.len() as u64)
                }
            }
            Body::Streaming => {
                if resp.chunked() {
                    resp.headers.remove(CONTENT_LENGTH);
                    if version != Version::HTTP_11 {
                        error!("Chunked transfer encoding is forbidden for {:?}", version);
                    }
                    if version == Version::HTTP_2 {
                        resp.headers.remove(TRANSFER_ENCODING);
                        TransferEncoding::eof()
                    } else {
                        resp.headers.insert(
                            TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
                        TransferEncoding::chunked()
                    }
                } else {
                    TransferEncoding::eof()
                }
            }
            Body::Upgrade => {
                if version == Version::HTTP_2 {
                    error!("Connection upgrade is forbidden for HTTP/2");
                } else {
                    resp.headers.insert(CONNECTION, HeaderValue::from_static("upgrade"));
                }
                TransferEncoding::eof()
            }
        };
        resp.replace_body(body);

        PayloadEncoder(
            match encoding {
                ContentEncoding::Deflate => ContentEncoder::Deflate(
                    DeflateEncoder::new(transfer, Compression::Default)),
                ContentEncoding::Gzip => ContentEncoder::Gzip(
                    GzEncoder::new(transfer, Compression::Default)),
                ContentEncoding::Br => ContentEncoder::Br(
                    BrotliEncoder::new(transfer, 5)),
                ContentEncoding::Identity => ContentEncoder::Identity(transfer),
                ContentEncoding::Auto =>
                    unreachable!()
            }
        )
    }
}

impl PayloadEncoder {

    pub fn len(&self) -> usize {
        self.0.get_ref().len()
    }

    pub fn get_mut(&mut self) -> &mut BytesMut {
        self.0.get_mut()
    }

    pub fn is_eof(&self) -> bool {
        self.0.is_eof()
    }

    pub fn write(&mut self, payload: &[u8]) -> Result<(), io::Error> {
        self.0.write(payload)
    }

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

    pub fn get_ref(&self) -> &BytesMut {
        match *self {
            ContentEncoder::Br(ref encoder) =>
                &encoder.get_ref().buffer,
            ContentEncoder::Deflate(ref encoder) =>
                &encoder.get_ref().buffer,
            ContentEncoder::Gzip(ref encoder) =>
                &encoder.get_ref().buffer,
            ContentEncoder::Identity(ref encoder) =>
                &encoder.buffer,
        }
    }

    pub fn get_mut(&mut self) -> &mut BytesMut {
        match *self {
            ContentEncoder::Br(ref mut encoder) =>
                &mut encoder.get_mut().buffer,
            ContentEncoder::Deflate(ref mut encoder) =>
                &mut encoder.get_mut().buffer,
            ContentEncoder::Gzip(ref mut encoder) =>
                &mut encoder.get_mut().buffer,
            ContentEncoder::Identity(ref mut encoder) =>
                &mut encoder.buffer,
        }
    }

    pub fn write_eof(&mut self) -> Result<(), io::Error> {
        let encoder = mem::replace(self, ContentEncoder::Identity(TransferEncoding::eof()));

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

    pub fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match *self {
            ContentEncoder::Br(ref mut encoder) => {
                match encoder.write(data) {
                    Ok(_) => {
                        encoder.flush()
                    },
                    Err(err) => {
                        trace!("Error decoding br encoding: {}", err);
                        Err(err)
                    },
                }
            },
            ContentEncoder::Gzip(ref mut encoder) => {
                match encoder.write(data) {
                    Ok(_) => {
                        encoder.flush()
                    },
                    Err(err) => {
                        trace!("Error decoding br encoding: {}", err);
                        Err(err)
                    },
                }
            }
            ContentEncoder::Deflate(ref mut encoder) => {
                match encoder.write(data) {
                    Ok(_) => {
                        encoder.flush()
                    },
                    Err(err) => {
                        trace!("Error decoding deflate encoding: {}", err);
                        Err(err)
                    },
                }
            }
            ContentEncoder::Identity(ref mut encoder) => {
                encoder.write_all(data)?;
                Ok(())
            }
        }
    }
}

/// Encoders to handle different Transfer-Encodings.
#[derive(Debug, Clone)]
pub(crate) struct TransferEncoding {
    kind: TransferEncodingKind,
    buffer: BytesMut,
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

    pub fn eof() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Eof,
            buffer: BytesMut::new(),
        }
    }

    pub fn chunked() -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Chunked(false),
            buffer: BytesMut::new(),
        }
    }

    pub fn length(len: u64) -> TransferEncoding {
        TransferEncoding {
            kind: TransferEncodingKind::Length(len),
            buffer: BytesMut::new(),
        }
    }

    pub fn is_eof(&self) -> bool {
        match self.kind {
            TransferEncodingKind::Eof => true,
            TransferEncodingKind::Chunked(ref eof) =>
                *eof,
            TransferEncodingKind::Length(ref remaining) =>
                *remaining == 0,
        }
    }

    /// Encode message. Return `EOF` state of encoder
    pub fn encode(&mut self, msg: &[u8]) -> bool {
        match self.kind {
            TransferEncodingKind::Eof => {
                self.buffer.extend(msg);
                msg.is_empty()
            },
            TransferEncodingKind::Chunked(ref mut eof) => {
                if *eof {
                    return true;
                }

                if msg.is_empty() {
                    *eof = true;
                    self.buffer.extend(b"0\r\n\r\n");
                } else {
                    write!(self.buffer, "{:X}\r\n", msg.len()).unwrap();
                    self.buffer.extend(msg);
                    self.buffer.extend(b"\r\n");
                }
                *eof
            },
            TransferEncodingKind::Length(ref mut remaining) => {
                if msg.is_empty() {
                    return *remaining == 0
                }
                let max = cmp::min(*remaining, msg.len() as u64);
                trace!("sized write = {}", max);
                self.buffer.extend(msg[..max as usize].as_ref());

                *remaining -= max as u64;
                trace!("encoded {} bytes, remaining = {}", max, remaining);
                *remaining == 0
            },
        }
    }

    /// Encode eof. Return `EOF` state of encoder
    pub fn encode_eof(&mut self) {
        match self.kind {
            TransferEncodingKind::Eof | TransferEncodingKind::Length(_) => (),
            TransferEncodingKind::Chunked(ref mut eof) => {
                if !*eof {
                    *eof = true;
                    self.buffer.extend(b"0\r\n\r\n");
                }
            },
        }
    }
}

impl io::Write for TransferEncoding {

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.encode(buf);
        Ok(buf.len())
    }

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

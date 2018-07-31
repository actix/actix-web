use std::io::{self, Write};

#[cfg(feature = "brotli")]
use brotli2::write::BrotliDecoder;
use bytes::{Bytes, BytesMut};
use error::PayloadError;
#[cfg(feature = "flate2")]
use flate2::write::{DeflateDecoder, GzDecoder};
use header::ContentEncoding;
use http::header::{HeaderMap, CONTENT_ENCODING};
use payload::{PayloadSender, PayloadStatus, PayloadWriter};

pub(crate) enum PayloadType {
    Sender(PayloadSender),
    Encoding(Box<EncodedPayload>),
}

impl PayloadType {
    #[cfg(any(feature = "brotli", feature = "flate2"))]
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
            ContentEncoding::Auto | ContentEncoding::Identity => {
                PayloadType::Sender(sender)
            }
            _ => PayloadType::Encoding(Box::new(EncodedPayload::new(sender, enc))),
        }
    }

    #[cfg(not(any(feature = "brotli", feature = "flate2")))]
    pub fn new(headers: &HeaderMap, sender: PayloadSender) -> PayloadType {
        PayloadType::Sender(sender)
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
        EncodedPayload {
            inner,
            error: false,
            payload: PayloadStream::new(enc),
        }
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
                }
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
            return;
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
    #[cfg(feature = "flate2")]
    Deflate(Box<DeflateDecoder<Writer>>),
    #[cfg(feature = "flate2")]
    Gzip(Box<GzDecoder<Writer>>),
    #[cfg(feature = "brotli")]
    Br(Box<BrotliDecoder<Writer>>),
    Identity,
}

pub(crate) struct Writer {
    buf: BytesMut,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            buf: BytesMut::with_capacity(8192),
        }
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
}

impl PayloadStream {
    pub fn new(enc: ContentEncoding) -> PayloadStream {
        let decoder = match enc {
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => {
                Decoder::Br(Box::new(BrotliDecoder::new(Writer::new())))
            }
            #[cfg(feature = "flate2")]
            ContentEncoding::Deflate => {
                Decoder::Deflate(Box::new(DeflateDecoder::new(Writer::new())))
            }
            #[cfg(feature = "flate2")]
            ContentEncoding::Gzip => {
                Decoder::Gzip(Box::new(GzDecoder::new(Writer::new())))
            }
            _ => Decoder::Identity,
        };
        PayloadStream { decoder }
    }
}

impl PayloadStream {
    pub fn feed_eof(&mut self) -> io::Result<Option<Bytes>> {
        match self.decoder {
            #[cfg(feature = "brotli")]
            Decoder::Br(ref mut decoder) => match decoder.finish() {
                Ok(mut writer) => {
                    let b = writer.take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(feature = "flate2")]
            Decoder::Gzip(ref mut decoder) => match decoder.try_finish() {
                Ok(_) => {
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(feature = "flate2")]
            Decoder::Deflate(ref mut decoder) => match decoder.try_finish() {
                Ok(_) => {
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            Decoder::Identity => Ok(None),
        }
    }

    pub fn feed_data(&mut self, data: Bytes) -> io::Result<Option<Bytes>> {
        match self.decoder {
            #[cfg(feature = "brotli")]
            Decoder::Br(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(feature = "flate2")]
            Decoder::Gzip(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            #[cfg(feature = "flate2")]
            Decoder::Deflate(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },
            Decoder::Identity => Ok(Some(data)),
        }
    }
}

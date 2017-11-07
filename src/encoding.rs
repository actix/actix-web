use std::{io, cmp};
use std::rc::Rc;
use std::cell::RefCell;
use std::io::{Read, Write};

use http::header::{HeaderMap, CONTENT_ENCODING};
use flate2::read::{GzDecoder};
use flate2::write::{DeflateDecoder};
use brotli2::write::BrotliDecoder;
use bytes::{Bytes, BytesMut, BufMut, Writer};

use payload::{PayloadSender, PayloadWriter, PayloadError};

/// Represents various types of connection
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
    Zlib(DeflateDecoder<BytesWriter>),
    Gzip(Option<GzDecoder<Wrapper>>),
    Br(Rc<RefCell<BytesMut>>, BrotliDecoder<WrapperRc>),
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


// should go after brotli2::write::BrotliDecoder::get_mut get implemented
#[derive(Debug)]
struct WrapperRc {
    buf: Rc<RefCell<BytesMut>>,
}

impl io::Write for WrapperRc {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.borrow_mut().extend(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct EncodedPayload {
    inner: PayloadSender,
    decoder: Decoder,
    dst: Writer<BytesMut>,
    error: bool,
}

impl EncodedPayload {
    pub fn new(inner: PayloadSender, enc: ContentEncoding) -> EncodedPayload {
        let dec = match enc {
            ContentEncoding::Deflate => Decoder::Zlib(
                DeflateDecoder::new(BytesWriter::default())),
            ContentEncoding::Gzip => Decoder::Gzip(None),
            ContentEncoding::Br => {
                let buf = Rc::new(RefCell::new(BytesMut::new()));
                let buf2 = Rc::clone(&buf);
                Decoder::Br(buf, BrotliDecoder::new(WrapperRc{buf: buf2}))
            }
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
            Decoder::Br(ref mut buf, ref mut decoder) => {
                match decoder.flush() {
                    Ok(_) => {
                        let b = buf.borrow_mut().take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        self.inner.feed_eof();
                        return
                    },
                    Err(err) => Some(err),
                }
            }

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
            }
            Decoder::Zlib(ref mut decoder) => {
                match decoder.flush() {
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
            Decoder::Br(ref mut buf, ref mut decoder) => {
                match decoder.write(&data) {
                    Ok(_) => {
                        let b = buf.borrow_mut().take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        return
                    },
                    Err(err) => {
                        trace!("Error decoding br encoding: {}", err);
                    },
                }
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

            Decoder::Zlib(ref mut decoder) => {
                match decoder.write(&data) {
                    Ok(_) => {
                        let b = decoder.get_mut().buf.take().freeze();
                        if !b.is_empty() {
                            self.inner.feed_data(b);
                        }
                        return
                    },
                    Err(err) => {
                        trace!("Error decoding deflate encoding: {}", err);
                    },
                }
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
        match self.decoder {
            Decoder::Br(ref buf, _) => {
                buf.borrow().len() + self.inner.capacity()
            }
            _ => {
                self.inner.capacity()
            }
        }
    }
}

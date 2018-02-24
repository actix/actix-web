use std::io;
use std::io::{Read, Write};
use bytes::{Bytes, BytesMut, BufMut};

use flate2::read::GzDecoder;
use flate2::write::DeflateDecoder;
use brotli2::write::BrotliDecoder;

use headers::ContentEncoding;
use server::encoding::{Decoder, Wrapper};


/// Payload wrapper with content decompression support
pub(crate) struct PayloadStream {
    decoder: Decoder,
    dst: BytesMut,
}

impl PayloadStream {
    pub fn new(enc: ContentEncoding) -> PayloadStream {
        let dec = match enc {
            ContentEncoding::Br => Decoder::Br(
                Box::new(BrotliDecoder::new(BytesMut::with_capacity(8192).writer()))),
            ContentEncoding::Deflate => Decoder::Deflate(
                Box::new(DeflateDecoder::new(BytesMut::with_capacity(8192).writer()))),
            ContentEncoding::Gzip => Decoder::Gzip(None),
            _ => Decoder::Identity,
        };
        PayloadStream{ decoder: dec, dst: BytesMut::new() }
    }
}

impl PayloadStream {

    pub fn feed_eof(&mut self) -> io::Result<Option<Bytes>> {
        match self.decoder {
            Decoder::Br(ref mut decoder) => {
                match decoder.finish() {
                    Ok(mut writer) => {
                        let b = writer.get_mut().take().freeze();
                        if !b.is_empty() {
                            Ok(Some(b))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(err) => Err(err),
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
                                    return Ok(Some(self.dst.take().freeze()))
                                } else {
                                    unsafe{self.dst.set_len(n)};
                                }
                            }
                            Err(err) => return Err(err),
                        }
                    }
                } else {
                    Ok(None)
                }
            },
            Decoder::Deflate(ref mut decoder) => {
                match decoder.try_finish() {
                    Ok(_) => {
                        let b = decoder.get_mut().get_mut().take().freeze();
                        if !b.is_empty() {
                            Ok(Some(b))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(err) => Err(err),
                }
            },
            Decoder::Identity => Ok(None),
        }
    }

    pub fn feed_data(&mut self, data: Bytes) -> io::Result<Option<Bytes>> {
        match self.decoder {
            Decoder::Br(ref mut decoder) => {
                match decoder.write(&data).and_then(|_| decoder.flush()) {
                    Ok(_) => {
                        let b = decoder.get_mut().get_mut().take().freeze();
                        if !b.is_empty() {
                            Ok(Some(b))
                        } else {
                            Ok(None)
                        }
                    },
                    Err(err) => Err(err)
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
                    match decoder.as_mut().as_mut().unwrap().read(unsafe{self.dst.bytes_mut()}) {
                        Ok(n) =>  {
                            if n == 0 {
                                return Ok(Some(self.dst.split_to(n).freeze()));
                            } else {
                                unsafe{self.dst.set_len(n)};
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            },
            Decoder::Deflate(ref mut decoder) => {
                match decoder.write(&data).and_then(|_| decoder.flush()) {
                    Ok(_) => {
                        let b = decoder.get_mut().get_mut().take().freeze();
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

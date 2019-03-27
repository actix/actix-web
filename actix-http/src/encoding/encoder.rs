//! Stream encoder
use std::io::{self, Write};

use bytes::Bytes;
use futures::{Async, Poll};

#[cfg(feature = "brotli")]
use brotli2::write::BrotliEncoder;
#[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
use flate2::write::{GzEncoder, ZlibEncoder};

use crate::body::{Body, BodyLength, MessageBody, ResponseBody};
use crate::http::header::{ContentEncoding, CONTENT_ENCODING};
use crate::http::{HeaderValue, HttpTryFrom, StatusCode};
use crate::{Error, Head, ResponseHead};

use super::Writer;

pub struct Encoder<B> {
    body: EncoderBody<B>,
    encoder: Option<ContentEncoder>,
}

impl<B: MessageBody> Encoder<B> {
    pub fn response(
        encoding: ContentEncoding,
        head: &mut ResponseHead,
        body: ResponseBody<B>,
    ) -> ResponseBody<Encoder<B>> {
        let has_ce = head.headers().contains_key(CONTENT_ENCODING);
        match body {
            ResponseBody::Other(b) => match b {
                Body::None => ResponseBody::Other(Body::None),
                Body::Empty => ResponseBody::Other(Body::Empty),
                Body::Bytes(buf) => {
                    if !(has_ce
                        || encoding == ContentEncoding::Identity
                        || encoding == ContentEncoding::Auto)
                    {
                        let mut enc = ContentEncoder::encoder(encoding).unwrap();

                        // TODO return error!
                        let _ = enc.write(buf.as_ref());
                        let body = enc.finish().unwrap();
                        update_head(encoding, head);
                        ResponseBody::Other(Body::Bytes(body))
                    } else {
                        ResponseBody::Other(Body::Bytes(buf))
                    }
                }
                Body::Message(stream) => {
                    if has_ce || head.status == StatusCode::SWITCHING_PROTOCOLS {
                        ResponseBody::Body(Encoder {
                            body: EncoderBody::Other(stream),
                            encoder: None,
                        })
                    } else {
                        update_head(encoding, head);
                        head.no_chunking = false;
                        ResponseBody::Body(Encoder {
                            body: EncoderBody::Other(stream),
                            encoder: ContentEncoder::encoder(encoding),
                        })
                    }
                }
            },
            ResponseBody::Body(stream) => {
                if has_ce || head.status == StatusCode::SWITCHING_PROTOCOLS {
                    ResponseBody::Body(Encoder {
                        body: EncoderBody::Body(stream),
                        encoder: None,
                    })
                } else {
                    update_head(encoding, head);
                    head.no_chunking = false;
                    ResponseBody::Body(Encoder {
                        body: EncoderBody::Body(stream),
                        encoder: ContentEncoder::encoder(encoding),
                    })
                }
            }
        }
    }
}

enum EncoderBody<B> {
    Body(B),
    Other(Box<dyn MessageBody>),
}

impl<B: MessageBody> MessageBody for Encoder<B> {
    fn length(&self) -> BodyLength {
        if self.encoder.is_none() {
            match self.body {
                EncoderBody::Body(ref b) => b.length(),
                EncoderBody::Other(ref b) => b.length(),
            }
        } else {
            BodyLength::Stream
        }
    }

    fn poll_next(&mut self) -> Poll<Option<Bytes>, Error> {
        loop {
            let result = match self.body {
                EncoderBody::Body(ref mut b) => b.poll_next()?,
                EncoderBody::Other(ref mut b) => b.poll_next()?,
            };
            match result {
                Async::NotReady => return Ok(Async::NotReady),
                Async::Ready(Some(chunk)) => {
                    if let Some(ref mut encoder) = self.encoder {
                        if encoder.write(&chunk)? {
                            return Ok(Async::Ready(Some(encoder.take())));
                        }
                    } else {
                        return Ok(Async::Ready(Some(chunk)));
                    }
                }
                Async::Ready(None) => {
                    if let Some(encoder) = self.encoder.take() {
                        let chunk = encoder.finish()?;
                        if chunk.is_empty() {
                            return Ok(Async::Ready(None));
                        } else {
                            return Ok(Async::Ready(Some(chunk)));
                        }
                    } else {
                        return Ok(Async::Ready(None));
                    }
                }
            }
        }
    }
}

fn update_head(encoding: ContentEncoding, head: &mut ResponseHead) {
    head.headers_mut().insert(
        CONTENT_ENCODING,
        HeaderValue::try_from(Bytes::from_static(encoding.as_str().as_bytes())).unwrap(),
    );
}

enum ContentEncoder {
    #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
    Deflate(ZlibEncoder<Writer>),
    #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
    Gzip(GzEncoder<Writer>),
    #[cfg(feature = "brotli")]
    Br(BrotliEncoder<Writer>),
}

impl ContentEncoder {
    fn encoder(encoding: ContentEncoding) -> Option<Self> {
        match encoding {
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoding::Deflate => Some(ContentEncoder::Deflate(ZlibEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoding::Gzip => Some(ContentEncoder::Gzip(GzEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => {
                Some(ContentEncoder::Br(BrotliEncoder::new(Writer::new(), 3)))
            }
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn take(&mut self) -> Bytes {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => encoder.get_mut().take(),
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().take(),
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().take(),
        }
    }

    fn finish(self) -> Result<Bytes, io::Error> {
        match self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Gzip(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Deflate(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<bool, io::Error> {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(!encoder.get_ref().buf.is_empty()),
                Err(err) => {
                    trace!("Error decoding br encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Gzip(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(!encoder.get_ref().buf.is_empty()),
                Err(err) => {
                    trace!("Error decoding gzip encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Deflate(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(!encoder.get_ref().buf.is_empty()),
                Err(err) => {
                    trace!("Error decoding deflate encoding: {}", err);
                    Err(err)
                }
            },
        }
    }
}

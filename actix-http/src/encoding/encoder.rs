//! Stream encoder
use std::io::{self, Write};

use actix_threadpool::{run, CpuFuture};
#[cfg(feature = "brotli")]
use brotli2::write::BrotliEncoder;
use bytes::Bytes;
#[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
use flate2::write::{GzEncoder, ZlibEncoder};
use futures::{Async, Future, Poll};

use crate::body::{Body, BodySize, MessageBody, ResponseBody};
use crate::http::header::{ContentEncoding, CONTENT_ENCODING};
use crate::http::{HeaderValue, HttpTryFrom, StatusCode};
use crate::{Error, ResponseHead};

use super::Writer;

const INPLACE: usize = 2049;

pub struct Encoder<B> {
    eof: bool,
    body: EncoderBody<B>,
    encoder: Option<ContentEncoder>,
    fut: Option<CpuFuture<ContentEncoder, io::Error>>,
}

impl<B: MessageBody> Encoder<B> {
    pub fn response(
        encoding: ContentEncoding,
        head: &mut ResponseHead,
        body: ResponseBody<B>,
    ) -> ResponseBody<Encoder<B>> {
        let can_encode = !(head.headers().contains_key(&CONTENT_ENCODING)
            || head.status == StatusCode::SWITCHING_PROTOCOLS
            || head.status == StatusCode::NO_CONTENT
            || encoding == ContentEncoding::Identity
            || encoding == ContentEncoding::Auto);

        let body = match body {
            ResponseBody::Other(b) => match b {
                Body::None => return ResponseBody::Other(Body::None),
                Body::Empty => return ResponseBody::Other(Body::Empty),
                Body::Bytes(buf) => {
                    if can_encode {
                        EncoderBody::Bytes(buf)
                    } else {
                        return ResponseBody::Other(Body::Bytes(buf));
                    }
                }
                Body::Message(stream) => EncoderBody::BoxedStream(stream),
            },
            ResponseBody::Body(stream) => EncoderBody::Stream(stream),
        };

        if can_encode {
            // Modify response body only if encoder is not None
            if let Some(enc) = ContentEncoder::encoder(encoding) {
                update_head(encoding, head);
                head.no_chunking(false);
                return ResponseBody::Body(Encoder {
                    body,
                    eof: false,
                    fut: None,
                    encoder: Some(enc),
                });
            }
        }
        ResponseBody::Body(Encoder {
            body,
            eof: false,
            fut: None,
            encoder: None,
        })
    }
}

enum EncoderBody<B> {
    Bytes(Bytes),
    Stream(B),
    BoxedStream(Box<dyn MessageBody>),
}

impl<B: MessageBody> MessageBody for Encoder<B> {
    fn size(&self) -> BodySize {
        if self.encoder.is_none() {
            match self.body {
                EncoderBody::Bytes(ref b) => b.size(),
                EncoderBody::Stream(ref b) => b.size(),
                EncoderBody::BoxedStream(ref b) => b.size(),
            }
        } else {
            BodySize::Stream
        }
    }

    fn poll_next(&mut self) -> Poll<Option<Bytes>, Error> {
        loop {
            if self.eof {
                return Ok(Async::Ready(None));
            }

            if let Some(ref mut fut) = self.fut {
                let mut encoder = futures::try_ready!(fut.poll());
                let chunk = encoder.take();
                self.encoder = Some(encoder);
                self.fut.take();
                if !chunk.is_empty() {
                    return Ok(Async::Ready(Some(chunk)));
                }
            }

            let result = match self.body {
                EncoderBody::Bytes(ref mut b) => {
                    if b.is_empty() {
                        Async::Ready(None)
                    } else {
                        Async::Ready(Some(std::mem::replace(b, Bytes::new())))
                    }
                }
                EncoderBody::Stream(ref mut b) => b.poll_next()?,
                EncoderBody::BoxedStream(ref mut b) => b.poll_next()?,
            };
            match result {
                Async::NotReady => return Ok(Async::NotReady),
                Async::Ready(Some(chunk)) => {
                    if let Some(mut encoder) = self.encoder.take() {
                        if chunk.len() < INPLACE {
                            encoder.write(&chunk)?;
                            let chunk = encoder.take();
                            self.encoder = Some(encoder);
                            if !chunk.is_empty() {
                                return Ok(Async::Ready(Some(chunk)));
                            }
                        } else {
                            self.fut = Some(run(move || {
                                encoder.write(&chunk)?;
                                Ok(encoder)
                            }));
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
                            self.eof = true;
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

    fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match *self {
            #[cfg(feature = "brotli")]
            ContentEncoder::Br(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding br encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Gzip(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding gzip encoding: {}", err);
                    Err(err)
                }
            },
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoder::Deflate(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding deflate encoding: {}", err);
                    Err(err)
                }
            },
        }
    }
}

//! Stream encoder
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::task::{Context, Poll};

use actix_rt::task::{spawn_blocking, JoinHandle};
use brotli2::write::BrotliEncoder;
use bytes::Bytes;
use flate2::write::{GzEncoder, ZlibEncoder};
use futures_core::ready;
use pin_project_lite::pin_project;

use crate::body::{Body, BodySize, MessageBody, ResponseBody};
use crate::http::header::{ContentEncoding, CONTENT_ENCODING};
use crate::http::{HeaderValue, StatusCode};
use crate::{Error, ResponseHead};

use super::Writer;
use crate::error::BlockingError;

const INPLACE: usize = 1024;

pin_project! {
    pub struct Encoder<B> {
        eof: bool,
        #[pin]
        body: EncoderBody<B>,
        encoder: Option<ContentEncoder>,
        fut: Option<JoinHandle<Result<ContentEncoder, io::Error>>>,
    }
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
            ResponseBody::Other { body } => match body {
                Body::None => return ResponseBody::Other { body: Body::None },
                Body::Empty => return ResponseBody::Other { body: Body::Empty },
                Body::Bytes(bytes) => {
                    if can_encode {
                        EncoderBody::Bytes { bytes }
                    } else {
                        return ResponseBody::Other {
                            body: Body::Bytes(bytes),
                        };
                    }
                }
                Body::Message(stream) => EncoderBody::BoxedStream { stream },
            },
            ResponseBody::Body { body } => EncoderBody::Stream { stream: body },
        };

        if can_encode {
            // Modify response body only if encoder is not None
            if let Some(enc) = ContentEncoder::encoder(encoding) {
                update_head(encoding, head);
                head.no_chunking(false);
                return ResponseBody::Body {
                    body: Encoder {
                        body,
                        eof: false,
                        fut: None,
                        encoder: Some(enc),
                    },
                };
            }
        }
        ResponseBody::Body {
            body: Encoder {
                body,
                eof: false,
                fut: None,
                encoder: None,
            },
        }
    }
}

pin_project! {
    #[project = EncoderBodyProj]
    enum EncoderBody<B> {
        Bytes { bytes: Bytes },
        Stream { #[pin] stream: B },
        BoxedStream { stream: Box<dyn MessageBody + Unpin> }
    }
}

impl<B: MessageBody> MessageBody for EncoderBody<B> {
    fn size(&self) -> BodySize {
        match self {
            EncoderBody::Bytes { ref bytes } => bytes.size(),
            EncoderBody::Stream { ref stream } => stream.size(),
            EncoderBody::BoxedStream { ref stream } => stream.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        match self.project() {
            EncoderBodyProj::Bytes { bytes } => {
                if bytes.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(std::mem::take(bytes))))
                }
            }
            EncoderBodyProj::Stream { stream } => stream.poll_next(cx),
            EncoderBodyProj::BoxedStream { stream } => {
                Pin::new(stream.as_mut()).poll_next(cx)
            }
        }
    }
}

impl<B: MessageBody> MessageBody for Encoder<B> {
    fn size(&self) -> BodySize {
        if self.encoder.is_none() {
            self.body.size()
        } else {
            BodySize::Stream
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Error>>> {
        let mut this = self.project();
        loop {
            if *this.eof {
                return Poll::Ready(None);
            }

            if let Some(ref mut fut) = this.fut {
                let mut encoder =
                    ready!(Pin::new(fut).poll(cx)).map_err(|_| BlockingError)??;
                let chunk = encoder.take();
                *this.encoder = Some(encoder);
                this.fut.take();
                if !chunk.is_empty() {
                    return Poll::Ready(Some(Ok(chunk)));
                }
            }

            let result = this.body.as_mut().poll_next(cx);

            match result {
                Poll::Ready(Some(Ok(chunk))) => {
                    if let Some(mut encoder) = this.encoder.take() {
                        if chunk.len() < INPLACE {
                            encoder.write(&chunk)?;
                            let chunk = encoder.take();
                            *this.encoder = Some(encoder);
                            if !chunk.is_empty() {
                                return Poll::Ready(Some(Ok(chunk)));
                            }
                        } else {
                            *this.fut = Some(spawn_blocking(move || {
                                encoder.write(&chunk)?;
                                Ok(encoder)
                            }));
                        }
                    } else {
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                }
                Poll::Ready(None) => {
                    if let Some(encoder) = this.encoder.take() {
                        let chunk = encoder.finish()?;
                        if chunk.is_empty() {
                            return Poll::Ready(None);
                        } else {
                            *this.eof = true;
                            return Poll::Ready(Some(Ok(chunk)));
                        }
                    } else {
                        return Poll::Ready(None);
                    }
                }
                val => return val,
            }
        }
    }
}

fn update_head(encoding: ContentEncoding, head: &mut ResponseHead) {
    head.headers_mut().insert(
        CONTENT_ENCODING,
        HeaderValue::from_static(encoding.as_str()),
    );
}

enum ContentEncoder {
    Deflate(ZlibEncoder<Writer>),
    Gzip(GzEncoder<Writer>),
    Br(BrotliEncoder<Writer>),
}

impl ContentEncoder {
    fn encoder(encoding: ContentEncoding) -> Option<Self> {
        match encoding {
            ContentEncoding::Deflate => Some(ContentEncoder::Deflate(ZlibEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            ContentEncoding::Gzip => Some(ContentEncoder::Gzip(GzEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            ContentEncoding::Br => {
                Some(ContentEncoder::Br(BrotliEncoder::new(Writer::new(), 3)))
            }
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn take(&mut self) -> Bytes {
        match *self {
            ContentEncoder::Br(ref mut encoder) => encoder.get_mut().take(),
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().take(),
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().take(),
        }
    }

    fn finish(self) -> Result<Bytes, io::Error> {
        match self {
            ContentEncoder::Br(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            ContentEncoder::Gzip(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            ContentEncoder::Deflate(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match *self {
            ContentEncoder::Br(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding br encoding: {}", err);
                    Err(err)
                }
            },
            ContentEncoder::Gzip(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding gzip encoding: {}", err);
                    Err(err)
                }
            },
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

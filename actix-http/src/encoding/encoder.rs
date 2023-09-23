//! Stream encoders.

use std::{
    error::Error as StdError,
    io::{self, Write as _},
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, Bytes};
use derive_more::Display;
#[cfg(feature = "compress-gzip")]
use flate2::write::{GzEncoder, ZlibEncoder};
use futures_core::ready;
use pin_project_lite::pin_project;
use tracing::trace;
#[cfg(feature = "compress-zstd")]
use zstd::stream::write::Encoder as ZstdEncoder;

use super::Writer;
use crate::{
    body::{self, BodySize, MessageBody},
    header::{self, ContentEncoding, HeaderValue, CONTENT_ENCODING},
    ResponseHead, StatusCode,
};

pin_project! {
    pub struct Encoder<B> {
        #[pin]
        body: EncoderBody<B>,
        encoder: Option<SelectedContentEncoder>,
        chunk_ready_to_encode: Option<Bytes>,
        eof: bool,
    }
}

impl<B: MessageBody> Encoder<B> {
    fn none() -> Self {
        Encoder {
            body: EncoderBody::None {
                body: body::None::new(),
            },
            encoder: None,
            chunk_ready_to_encode: None,
            eof: true,
        }
    }

    pub fn response(encoding: ContentEncoding, head: &mut ResponseHead, body: B) -> Self {
        // no need to compress an empty body
        if matches!(body.size(), BodySize::None) {
            return Self::none();
        }

        let should_encode = !(head.headers().contains_key(&CONTENT_ENCODING)
            || head.status == StatusCode::SWITCHING_PROTOCOLS
            || head.status == StatusCode::NO_CONTENT
            || encoding == ContentEncoding::Identity);

        let body = match body.try_into_bytes() {
            Ok(body) => EncoderBody::Full { body },
            Err(body) => EncoderBody::Stream { body },
        };

        if should_encode {
            // wrap body only if encoder is feature-enabled
            if let Some(selected_encoder) = ContentEncoder::select(encoding) {
                update_head(encoding, head);

                return Encoder {
                    body,
                    encoder: Some(selected_encoder),
                    chunk_ready_to_encode: None,
                    eof: false,
                };
            }
        }

        Encoder {
            body,
            encoder: None,
            chunk_ready_to_encode: None,
            eof: false,
        }
    }

    pub fn with_encode_chunk_size(mut self, size: usize) -> Self {
        if size > 0 {
            if let Some(selected_encoder) = self.encoder.as_mut() {
                selected_encoder.preferred_chunk_size = size;
            }
        }
        self
    }
}

pin_project! {
    #[project = EncoderBodyProj]
    enum EncoderBody<B> {
        None { body: body::None },
        Full { body: Bytes },
        Stream { #[pin] body: B },
    }
}

impl<B> MessageBody for EncoderBody<B>
where
    B: MessageBody,
{
    type Error = EncoderError;

    #[inline]
    fn size(&self) -> BodySize {
        match self {
            EncoderBody::None { body } => body.size(),
            EncoderBody::Full { body } => body.size(),
            EncoderBody::Stream { body } => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.project() {
            EncoderBodyProj::None { body } => {
                Pin::new(body).poll_next(cx).map_err(|err| match err {})
            }
            EncoderBodyProj::Full { body } => {
                Pin::new(body).poll_next(cx).map_err(|err| match err {})
            }
            EncoderBodyProj::Stream { body } => body
                .poll_next(cx)
                .map_err(|err| EncoderError::Body(err.into())),
        }
    }

    #[inline]
    fn try_into_bytes(self) -> Result<Bytes, Self>
    where
        Self: Sized,
    {
        match self {
            EncoderBody::None { body } => Ok(body.try_into_bytes().unwrap()),
            EncoderBody::Full { body } => Ok(body.try_into_bytes().unwrap()),
            _ => Err(self),
        }
    }
}

impl<B> MessageBody for Encoder<B>
where
    B: MessageBody,
{
    type Error = EncoderError;

    #[inline]
    fn size(&self) -> BodySize {
        if self.encoder.is_some() {
            BodySize::Stream
        } else {
            self.body.size()
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        let mut this = self.project();

        loop {
            if *this.eof {
                return Poll::Ready(None);
            }

            if let Some(chunk) = this.chunk_ready_to_encode.as_mut() {
                let selected_encoder = this.encoder.as_mut().expect(
                    "when chunk_ready_to_encode is presented the encoder is expected to be presented as well",
                );
                let encode_len = chunk.len().min(selected_encoder.preferred_chunk_size);
                selected_encoder
                    .content_encoder
                    .write(&chunk[..encode_len])
                    .map_err(EncoderError::Io)?;
                chunk.advance(encode_len);

                if chunk.is_empty() {
                    *this.chunk_ready_to_encode = None;
                }

                let encoded_chunk = selected_encoder.content_encoder.take();
                if !encoded_chunk.is_empty() {
                    return Poll::Ready(Some(Ok(encoded_chunk)));
                }

                if this.chunk_ready_to_encode.is_some() {
                    // Yield execution to give chance other futures to execute
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
            }

            let result = ready!(this.body.as_mut().poll_next(cx));

            match result {
                Some(Err(err)) => return Poll::Ready(Some(Err(err))),

                Some(Ok(chunk)) => {
                    if this.encoder.is_none() {
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                    *this.chunk_ready_to_encode = Some(chunk);
                }

                None => {
                    if let Some(selected_encoder) = this.encoder.take() {
                        let chunk = selected_encoder
                            .content_encoder
                            .finish()
                            .map_err(EncoderError::Io)?;

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
            }
        }
    }

    #[inline]
    fn try_into_bytes(mut self) -> Result<Bytes, Self>
    where
        Self: Sized,
    {
        if self.encoder.is_some() {
            Err(self)
        } else {
            match self.body.try_into_bytes() {
                Ok(body) => Ok(body),
                Err(body) => {
                    self.body = body;
                    Err(self)
                }
            }
        }
    }
}

fn update_head(encoding: ContentEncoding, head: &mut ResponseHead) {
    head.headers_mut()
        .insert(header::CONTENT_ENCODING, encoding.to_header_value());
    head.headers_mut()
        .append(header::VARY, HeaderValue::from_static("accept-encoding"));

    head.no_chunking(false);
}

enum ContentEncoder {
    #[cfg(feature = "compress-gzip")]
    Deflate(ZlibEncoder<Writer>),

    #[cfg(feature = "compress-gzip")]
    Gzip(GzEncoder<Writer>),

    #[cfg(feature = "compress-brotli")]
    Brotli(Box<brotli::CompressorWriter<Writer>>),

    // Wwe need explicit 'static lifetime here because ZstdEncoder needs a lifetime argument and we
    // use `spawn_blocking` in `Encoder::poll_next` that requires `FnOnce() -> R + Send + 'static`.
    #[cfg(feature = "compress-zstd")]
    Zstd(ZstdEncoder<'static, Writer>),
}

struct SelectedContentEncoder {
    content_encoder: ContentEncoder,
    preferred_chunk_size: usize,
}

impl ContentEncoder {
    fn select(encoding: ContentEncoding) -> Option<SelectedContentEncoder> {
        // Chunk size picked as max chunk size which took less that 50 µs to compress on "cargo bench --bench compression-chunk-size"

        // Rust 1.72 linux/arm64 in Docker on Apple M2 Pro: "time to compress chunk/deflate-16384"  time: [39.114 µs 39.283 µs 39.457 µs]
        const MAX_DEFLATE_CHUNK_SIZE: usize = 16384;
        // Rust 1.72 linux/arm64 in Docker on Apple M2 Pro: "time to compress chunk/gzip-16384"     time: [40.121 µs 40.340 µs 40.566 µs]
        const MAX_GZIP_CHUNK_SIZE: usize = 16384;
        // Rust 1.72 linux/arm64 in Docker on Apple M2 Pro: "time to compress chunk/br-8192"        time: [46.076 µs 46.208 µs 46.343 µs]
        const MAX_BROTLI_CHUNK_SIZE: usize = 8192;
        // Rust 1.72 linux/arm64 in Docker on Apple M2 Pro: "time to compress chunk/zstd-16384"     time: [32.872 µs 32.967 µs 33.068 µs]
        const MAX_ZSTD_CHUNK_SIZE: usize = 16384;

        match encoding {
            #[cfg(feature = "compress-gzip")]
            ContentEncoding::Deflate => Some(SelectedContentEncoder {
                content_encoder: ContentEncoder::Deflate(ZlibEncoder::new(
                    Writer::new(),
                    flate2::Compression::fast(),
                )),
                preferred_chunk_size: MAX_DEFLATE_CHUNK_SIZE,
            }),

            #[cfg(feature = "compress-gzip")]
            ContentEncoding::Gzip => Some(SelectedContentEncoder {
                content_encoder: ContentEncoder::Gzip(GzEncoder::new(
                    Writer::new(),
                    flate2::Compression::fast(),
                )),
                preferred_chunk_size: MAX_GZIP_CHUNK_SIZE,
            }),

            #[cfg(feature = "compress-brotli")]
            ContentEncoding::Brotli => Some(SelectedContentEncoder {
                content_encoder: ContentEncoder::Brotli(new_brotli_compressor()),
                preferred_chunk_size: MAX_BROTLI_CHUNK_SIZE,
            }),

            #[cfg(feature = "compress-zstd")]
            ContentEncoding::Zstd => {
                let encoder = ZstdEncoder::new(Writer::new(), 3).ok()?;
                Some(SelectedContentEncoder {
                    content_encoder: ContentEncoder::Zstd(encoder),
                    preferred_chunk_size: MAX_ZSTD_CHUNK_SIZE,
                })
            }

            _ => None,
        }
    }

    #[inline]
    pub(crate) fn take(&mut self) -> Bytes {
        match *self {
            #[cfg(feature = "compress-brotli")]
            ContentEncoder::Brotli(ref mut encoder) => encoder.get_mut().take(),

            #[cfg(feature = "compress-gzip")]
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().take(),

            #[cfg(feature = "compress-gzip")]
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().take(),

            #[cfg(feature = "compress-zstd")]
            ContentEncoder::Zstd(ref mut encoder) => encoder.get_mut().take(),
        }
    }

    fn finish(self) -> Result<Bytes, io::Error> {
        match self {
            #[cfg(feature = "compress-brotli")]
            ContentEncoder::Brotli(mut encoder) => match encoder.flush() {
                Ok(()) => Ok(encoder.into_inner().buf.freeze()),
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-gzip")]
            ContentEncoder::Gzip(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-gzip")]
            ContentEncoder::Deflate(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-zstd")]
            ContentEncoder::Zstd(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match *self {
            #[cfg(feature = "compress-brotli")]
            ContentEncoder::Brotli(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding br encoding: {}", err);
                    Err(err)
                }
            },

            #[cfg(feature = "compress-gzip")]
            ContentEncoder::Gzip(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding gzip encoding: {}", err);
                    Err(err)
                }
            },

            #[cfg(feature = "compress-gzip")]
            ContentEncoder::Deflate(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding deflate encoding: {}", err);
                    Err(err)
                }
            },

            #[cfg(feature = "compress-zstd")]
            ContentEncoder::Zstd(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding ztsd encoding: {}", err);
                    Err(err)
                }
            },
        }
    }
}

#[cfg(feature = "compress-brotli")]
fn new_brotli_compressor() -> Box<brotli::CompressorWriter<Writer>> {
    Box::new(brotli::CompressorWriter::new(
        Writer::new(),
        32 * 1024, // 32 KiB buffer
        3,         // BROTLI_PARAM_QUALITY
        22,        // BROTLI_PARAM_LGWIN
    ))
}

#[derive(Debug, Display)]
#[non_exhaustive]
pub enum EncoderError {
    /// Wrapped body stream error.
    #[display(fmt = "body")]
    Body(Box<dyn StdError>),

    /// Generic I/O error.
    #[display(fmt = "io")]
    Io(io::Error),
}

impl StdError for EncoderError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            EncoderError::Body(err) => Some(&**err),
            EncoderError::Io(err) => Some(err),
        }
    }
}

impl From<EncoderError> for crate::Error {
    fn from(err: EncoderError) -> Self {
        crate::Error::new_encoder().with_cause(err)
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use rand::{seq::SliceRandom, Rng};

    use super::*;

    static EMPTY_BODY: &[u8] = &[];

    static SHORT_BODY: &[u8] = &[1, 2, 3, 4, 6, 7, 8];

    static LONG_BODY: &[u8] = include_bytes!("encoder.rs");

    static BODIES: &[&[u8]] = &[EMPTY_BODY, SHORT_BODY, LONG_BODY];

    async fn test_compression_of_conentent_enconding(encoding: ContentEncoding, body: &[u8]) {
        let mut head = ResponseHead::new(StatusCode::OK);
        let body_to_compress = {
            let mut body = BytesMut::from(body);
            body.shuffle(&mut rand::thread_rng());
            body.freeze()
        };
        let compressed_body = Encoder::response(encoding, &mut head, body_to_compress.clone())
            .with_encode_chunk_size(rand::thread_rng().gen_range(32..128));

        let SelectedContentEncoder {
            content_encoder: mut compressor,
            preferred_chunk_size: _,
        } = ContentEncoder::select(encoding).unwrap();
        compressor.write(&body_to_compress).unwrap();
        let reference_compressed_bytes = compressor.finish().unwrap();

        let compressed_bytes =
            body::to_bytes_limited(compressed_body, 256 + body_to_compress.len())
                .await
                .unwrap()
                .unwrap();

        assert_eq!(reference_compressed_bytes, compressed_bytes);
    }

    #[actix_rt::test]
    #[cfg(feature = "compress-gzip")]
    async fn test_gzip_compression_in_chunks_is_the_same_as_whole_chunk_compression() {
        for body in BODIES {
            test_compression_of_conentent_enconding(ContentEncoding::Gzip, body).await;
        }
    }

    #[actix_rt::test]
    #[cfg(feature = "compress-gzip")]
    async fn test_deflate_compression_in_chunks_is_the_same_as_whole_chunk_compression() {
        for body in BODIES {
            test_compression_of_conentent_enconding(ContentEncoding::Deflate, body).await;
        }
    }

    #[actix_rt::test]
    #[cfg(feature = "compress-brotli")]
    async fn test_brotli_compression_in_chunks_is_the_same_as_whole_chunk_compression() {
        for body in BODIES {
            test_compression_of_conentent_enconding(ContentEncoding::Brotli, body).await;
        }
    }

    #[actix_rt::test]
    #[cfg(feature = "compress-zstd")]
    async fn test_zstd_compression_in_chunks_is_the_same_as_whole_chunk_compression() {
        for body in BODIES {
            test_compression_of_conentent_enconding(ContentEncoding::Zstd, body).await;
        }
    }
}

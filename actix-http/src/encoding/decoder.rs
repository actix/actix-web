//! Stream decoders.

use std::{
    future::Future,
    io::{self, Write as _},
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
#[cfg(feature = "compress-gzip")]
use flate2::write::{GzDecoder, ZlibDecoder};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;
use tokio::task::{spawn_blocking, JoinHandle};
#[cfg(feature = "compress-zstd")]
use zstd::stream::write::Decoder as ZstdDecoder;

use crate::{
    encoding::Writer,
    error::PayloadError,
    header::{ContentEncoding, HeaderMap, CONTENT_ENCODING},
};

const MAX_CHUNK_SIZE_DECODE_IN_PLACE: usize = 2049;

pin_project! {
    /// Payload stream that decompresses chunks using a content encoding.
    ///
    /// Yields decoded [`Bytes`] as they become available. Errors from the underlying stream are
    /// passed through; decompression errors are returned as [`PayloadError::Io`].
    ///
    /// Exceeding the configured decoded byte limit returns [`PayloadError::Overflow`].
    ///
    /// See the [module-level docs](crate::encoding) for supported encodings and runtime requirements.
    pub struct Decoder<S> {
        decoder: Option<ContentDecoder>,
        #[pin]
        stream: S,
        eof: bool,
        fut: Option<JoinHandle<io::Result<(Option<Bytes>, ContentDecoder)>>>,
    }
}

impl<S> Decoder<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    /// Constructs a decoder with the given content encoding.
    ///
    /// If `encoding` is [`ContentEncoding::Identity`] or its codec feature is disabled, the stream
    /// is passed through unchanged.
    #[inline]
    pub fn new(stream: S, encoding: ContentEncoding) -> Decoder<S> {
        #[cfg(feature = "compress-brotli")]
        const DEFAULT_DECODER_CAPACITY: usize = 8 * 1024; // 8 KiB

        let decoder = match encoding {
            #[cfg(feature = "compress-brotli")]
            ContentEncoding::Brotli => {
                let mut br =
                    brotli::DecompressorWriter::new(Writer::new(), DEFAULT_DECODER_CAPACITY);
                br.set_parameter(
                    brotli_decompressor::BrotliDecoderParameter::BROTLI_DECODER_PARAM_LARGE_WINDOW,
                    0,
                );

                Some(ContentDecoder::Brotli(Box::new(br)))
            }

            #[cfg(feature = "compress-gzip")]
            ContentEncoding::Deflate => Some(ContentDecoder::Deflate(Box::new(ZlibDecoder::new(
                Writer::new(),
            )))),

            #[cfg(feature = "compress-gzip")]
            ContentEncoding::Gzip => Some(ContentDecoder::Gzip(Box::new(GzDecoder::new(
                Writer::new(),
            )))),

            #[cfg(feature = "compress-zstd")]
            ContentEncoding::Zstd => Some(ContentDecoder::Zstd(Box::new(
                ZstdDecoder::new(Writer::new()).expect(
                    "Failed to create zstd decoder. This is a bug. \
                         Please report it to the actix-web repository.",
                ),
            ))),

            _ => None,
        };

        Decoder {
            decoder,
            stream,
            fut: None,
            eof: false,
        }
    }

    /// Set the maximum total number of decoded bytes.
    ///
    /// Decoding stops with [`PayloadError::Overflow`] if the limit is exceeded.
    ///
    /// Limit must be set before the stream is first polled.
    ///
    /// By default, decoded output is unlimited.
    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(ref mut decoder) = self.decoder {
            decoder.set_limit(limit);
        }

        self
    }

    /// Constructs a decoder using the `Content-Encoding` header.
    ///
    /// If the header is missing, invalid, or unrecognized, the stream is passed through unchanged.
    /// A recognized encoding whose codec feature is disabled also passes through unchanged.
    ///
    /// Only a single encoding is supported. Comma-separated encoding lists are treated as
    /// unrecognized values. If there are multiple `Content-Encoding` header values, only the first
    /// is used.
    #[inline]
    pub fn from_headers(stream: S, headers: &HeaderMap) -> Decoder<S> {
        // check content-encoding
        let encoding = headers
            .get(&CONTENT_ENCODING)
            .and_then(|val| val.to_str().ok())
            .and_then(|x| x.parse().ok())
            .unwrap_or(ContentEncoding::Identity);

        Self::new(stream, encoding)
    }
}

impl<S> Stream for Decoder<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            // Receive chunk and decoder state back from thread-offloaded decode task.
            if let Some(ref mut fut) = this.fut {
                let result = ready!(Pin::new(fut).poll(cx))
                    .map_err(|_| {
                        PayloadError::Io(io::Error::other(
                            "Blocking task was cancelled unexpectedly",
                        ))
                    })
                    .and_then(|result| result.map_err(from_decoder_error));

                // Offload task completed; throw away join handle.
                this.fut.take();

                let (chunk, decoder) = match result {
                    Ok(result) => result,
                    Err(err) => {
                        *this.eof = true;
                        return Poll::Ready(Some(Err(err)));
                    }
                };

                *this.decoder = Some(decoder);

                if let Some(chunk) = chunk {
                    return Poll::Ready(Some(Ok(chunk)));
                }
            }

            if *this.eof {
                return Poll::Ready(None);
            }

            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(Err(err)) => return Poll::Ready(Some(Err(err))),

                Some(Ok(chunk)) => {
                    if let Some(mut decoder) = this.decoder.take() {
                        if chunk.len() < MAX_CHUNK_SIZE_DECODE_IN_PLACE {
                            let chunk = match decoder.feed_data(chunk) {
                                Ok(chunk) => chunk,
                                Err(err) => {
                                    *this.eof = true;
                                    return Poll::Ready(Some(Err(from_decoder_error(err))));
                                }
                            };

                            *this.decoder = Some(decoder);

                            if let Some(chunk) = chunk {
                                return Poll::Ready(Some(Ok(chunk)));
                            }
                        } else {
                            *this.fut = Some(spawn_blocking(move || {
                                let chunk = decoder.feed_data(chunk)?;
                                Ok((chunk, decoder))
                            }));
                        }

                        continue;
                    } else {
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                }

                None => {
                    *this.eof = true;

                    return if let Some(mut decoder) = this.decoder.take() {
                        match decoder.feed_eof() {
                            Ok(Some(res)) => Poll::Ready(Some(Ok(res))),
                            Ok(None) => Poll::Ready(None),
                            Err(err) => Poll::Ready(Some(Err(from_decoder_error(err)))),
                        }
                    } else {
                        Poll::Ready(None)
                    };
                }
            }
        }
    }
}

fn from_decoder_error(err: io::Error) -> PayloadError {
    match err.kind() {
        io::ErrorKind::FileTooLarge => PayloadError::Overflow,
        _ => err.into(),
    }
}

enum ContentDecoder {
    #[cfg(feature = "compress-gzip")]
    Deflate(Box<ZlibDecoder<Writer>>),

    #[cfg(feature = "compress-gzip")]
    Gzip(Box<GzDecoder<Writer>>),

    #[cfg(feature = "compress-brotli")]
    Brotli(Box<brotli::DecompressorWriter<Writer>>),

    // We need 'static here because:
    // - ZstdDecoder needs a lifetime argument;
    // - we use `spawn_blocking` in `Decoder::poll_next` that requires `FnOnce() -> R + Send + 'static`.
    #[cfg(feature = "compress-zstd")]
    Zstd(Box<ZstdDecoder<'static, Writer>>),
}

impl ContentDecoder {
    fn set_limit(&mut self, limit: usize) {
        match self {
            #[cfg(feature = "compress-brotli")]
            ContentDecoder::Brotli(ref mut decoder) => decoder.get_mut().set_limit(limit),

            #[cfg(feature = "compress-gzip")]
            ContentDecoder::Gzip(ref mut decoder) => decoder.get_mut().set_limit(limit),

            #[cfg(feature = "compress-gzip")]
            ContentDecoder::Deflate(ref mut decoder) => decoder.get_mut().set_limit(limit),

            #[cfg(feature = "compress-zstd")]
            ContentDecoder::Zstd(ref mut decoder) => decoder.get_mut().set_limit(limit),
        }
    }

    fn feed_eof(&mut self) -> io::Result<Option<Bytes>> {
        match self {
            #[cfg(feature = "compress-brotli")]
            ContentDecoder::Brotli(ref mut decoder) => match decoder.flush() {
                Ok(()) => {
                    let b = decoder.get_mut().take();

                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-gzip")]
            ContentDecoder::Gzip(ref mut decoder) => match decoder.try_finish() {
                Ok(_) => {
                    let b = decoder.get_mut().take();

                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-gzip")]
            ContentDecoder::Deflate(ref mut decoder) => match decoder.try_finish() {
                Ok(_) => {
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-zstd")]
            ContentDecoder::Zstd(ref mut decoder) => match decoder.flush() {
                Ok(_) => {
                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },
        }
    }

    fn feed_data(&mut self, data: Bytes) -> io::Result<Option<Bytes>> {
        match self {
            #[cfg(feature = "compress-brotli")]
            ContentDecoder::Brotli(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();

                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-gzip")]
            ContentDecoder::Gzip(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;
                    let b = decoder.get_mut().take();

                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-gzip")]
            ContentDecoder::Deflate(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;

                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },

            #[cfg(feature = "compress-zstd")]
            ContentDecoder::Zstd(ref mut decoder) => match decoder.write_all(&data) {
                Ok(_) => {
                    decoder.flush()?;

                    let b = decoder.get_mut().take();
                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => Err(err),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{stream, StreamExt as _};
    use rand::RngExt as _;

    use super::*;

    const ENCODINGS: &[ContentEncoding] = &[
        #[cfg(feature = "compress-brotli")]
        ContentEncoding::Brotli,
        #[cfg(feature = "compress-gzip")]
        ContentEncoding::Gzip,
        #[cfg(feature = "compress-gzip")]
        ContentEncoding::Deflate,
        #[cfg(feature = "compress-zstd")]
        ContentEncoding::Zstd,
    ];

    #[cfg(feature = "compress-brotli")]
    #[tokio::test(flavor = "local")]
    async fn brotli_large_window_is_rejected() {
        let data = vec![b'a'; 4096];
        let params = brotli::enc::BrotliEncoderParams {
            large_window: true,
            lgwin: 25,
            ..Default::default()
        };
        let mut compressed = Vec::new();
        brotli::BrotliCompress(&mut &data[..], &mut compressed, &params).unwrap();

        assert_eq!(compressed[0], 0x11);

        let mut decoded = Vec::new();
        brotli::BrotliDecompress(&mut &compressed[..], &mut decoded).unwrap();
        assert_eq!(decoded, data);

        let stream = stream::iter([Ok(Bytes::from(compressed))]);
        let mut decoder = Decoder::new(stream, ContentEncoding::Brotli);

        assert!(matches!(
            decoder.next().await,
            Some(Err(PayloadError::Incomplete(Some(_))))
        ));
        assert!(decoder.next().await.is_none());
    }

    fn encode(data: &[u8], encoding: ContentEncoding) -> Bytes {
        match encoding {
            #[cfg(feature = "compress-brotli")]
            ContentEncoding::Brotli => {
                let mut encoder = brotli::CompressorWriter::new(Vec::new(), 4096, 3, 22);
                encoder.write_all(data).unwrap();
                encoder.into_inner().into()
            }

            #[cfg(feature = "compress-gzip")]
            ContentEncoding::Gzip => {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(data).unwrap();
                encoder.finish().unwrap().into()
            }

            #[cfg(feature = "compress-gzip")]
            ContentEncoding::Deflate => {
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(data).unwrap();
                encoder.finish().unwrap().into()
            }

            #[cfg(feature = "compress-zstd")]
            ContentEncoding::Zstd => zstd::stream::encode_all(data, 3).unwrap().into(),

            ContentEncoding::Identity => Bytes::copy_from_slice(data),
        }
    }

    #[tokio::test(flavor = "local")]
    async fn decoded_output_is_bounded() {
        let data = vec![b'a'; 1024 * 1024];

        for &encoding in ENCODINGS {
            let compressed = encode(&data, encoding);
            let stream = stream::iter([Ok(compressed)]);
            let mut decoder = Decoder::new(stream, encoding).limit(1024);

            assert!(
                matches!(decoder.next().await, Some(Err(PayloadError::Overflow))),
                "{encoding:?} should report overflow"
            );
            assert!(
                decoder.next().await.is_none(),
                "{encoding:?} should end after overflow"
            );
        }
    }

    #[tokio::test(flavor = "local")]
    async fn blocking_decode_reports_overflow() {
        let data_that_compresses_poorly = rand::rng()
            .sample_iter(rand::distr::StandardUniform)
            .take(65_536)
            .collect::<Vec<u8>>();

        for &encoding in ENCODINGS {
            let compressed = encode(&data_that_compresses_poorly, encoding);

            assert!(
                compressed.len() >= MAX_CHUNK_SIZE_DECODE_IN_PLACE,
                "{encoding:?} should use the blocking decode path"
            );

            let stream = stream::iter([Ok(compressed)]);
            let mut decoder = Decoder::new(stream, encoding).limit(1024);

            assert!(
                matches!(decoder.next().await, Some(Err(PayloadError::Overflow))),
                "{encoding:?} should report overflow"
            );
            assert!(
                decoder.next().await.is_none(),
                "{encoding:?} should end after overflow"
            );
        }
    }

    #[tokio::test(flavor = "local")]
    async fn decoded_output_at_limit_is_complete() {
        let data = vec![b'a'; 1024];

        for &encoding in ENCODINGS {
            let compressed = encode(&data, encoding);
            let stream = stream::iter(
                compressed
                    .chunks(8)
                    .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                    .collect::<Vec<_>>(),
            );

            let mut decoder = Decoder::new(stream, encoding).limit(data.len());
            let mut output = Vec::new();

            while let Some(chunk) = decoder.next().await {
                output.extend_from_slice(&chunk.unwrap());
            }

            assert_eq!(
                output, data,
                "{encoding:?} should decode the complete body at the limit"
            );
        }
    }

    #[tokio::test(flavor = "local")]
    async fn decoded_limit_applies_across_chunks() {
        let data = vec![b'a'; 1024 * 1024];

        for &encoding in ENCODINGS {
            let compressed = encode(&data, encoding);
            let chunks = compressed
                .chunks(16)
                .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>();

            let mut decoder = Decoder::new(stream::iter(chunks), encoding).limit(200_000);
            let mut size = 0;

            loop {
                match decoder.next().await {
                    Some(Ok(chunk)) => {
                        size += chunk.len();
                        assert!(
                            size <= 200_000,
                            "{encoding:?} should keep total decoded output within the limit"
                        );
                    }
                    Some(Err(PayloadError::Overflow)) => break,
                    result => panic!("{encoding:?} should report overflow, got {result:?}"),
                }
            }

            assert!(
                decoder.next().await.is_none(),
                "{encoding:?} should end after overflow"
            );
        }
    }

    #[tokio::test(flavor = "local")]
    async fn zero_decoded_limit_rejects_nonempty_body() {
        for &encoding in ENCODINGS {
            let compressed = encode(b"a", encoding);
            let mut decoder = Decoder::new(stream::iter([Ok(compressed)]), encoding).limit(0);
            assert!(
                matches!(decoder.next().await, Some(Err(PayloadError::Overflow))),
                "{encoding:?} should report overflow"
            );
        }
    }

    #[test]
    fn io_errors_are_not_classified_as_overflow() {
        assert!(matches!(
            from_decoder_error(io::Error::other("codec error")),
            PayloadError::Incomplete(Some(_))
        ));
    }
}

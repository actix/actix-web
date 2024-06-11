//! Stream decoders.

use std::{
    future::Future,
    io::{self, Write as _},
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::task::{spawn_blocking, JoinHandle};
use bytes::Bytes;
#[cfg(feature = "compress-gzip")]
use flate2::write::{GzDecoder, ZlibDecoder};
use futures_core::{ready, Stream};
#[cfg(feature = "compress-zstd")]
use zstd::stream::write::Decoder as ZstdDecoder;

use crate::{
    encoding::Writer,
    error::PayloadError,
    header::{ContentEncoding, HeaderMap, CONTENT_ENCODING},
};

const MAX_CHUNK_SIZE_DECODE_IN_PLACE: usize = 2049;

pin_project_lite::pin_project! {
    pub struct Decoder<S> {
        decoder: Option<ContentDecoder>,
        #[pin]
        stream: S,
        eof: bool,
        fut: Option<JoinHandle<Result<(Option<Bytes>, ContentDecoder), io::Error>>>,
    }
}

impl<S> Decoder<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    /// Construct a decoder.
    #[inline]
    pub fn new(stream: S, encoding: ContentEncoding) -> Decoder<S> {
        let decoder = match encoding {
            #[cfg(feature = "compress-brotli")]
            ContentEncoding::Brotli => Some(ContentDecoder::Brotli(Box::new(
                brotli::DecompressorWriter::new(Writer::new(), 8_096),
            ))),

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

    /// Construct decoder based on headers.
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
            if let Some(ref mut fut) = this.fut {
                let (chunk, decoder) = ready!(Pin::new(fut).poll(cx)).map_err(|_| {
                    PayloadError::Io(io::Error::new(
                        io::ErrorKind::Other,
                        "Blocking task was cancelled unexpectedly",
                    ))
                })??;

                *this.decoder = Some(decoder);
                this.fut.take();

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
                            let chunk = decoder.feed_data(chunk)?;
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
                            Err(err) => Poll::Ready(Some(Err(err.into()))),
                        }
                    } else {
                        Poll::Ready(None)
                    };
                }
            }
        }
    }
}

enum ContentDecoder {
    #[cfg(feature = "compress-gzip")]
    Deflate(Box<ZlibDecoder<Writer>>),

    #[cfg(feature = "compress-gzip")]
    Gzip(Box<GzDecoder<Writer>>),

    #[cfg(feature = "compress-brotli")]
    Brotli(Box<brotli::DecompressorWriter<Writer>>),

    // We need explicit 'static lifetime here because ZstdDecoder need lifetime
    // argument, and we use `spawn_blocking` in `Decoder::poll_next` that require `FnOnce() -> R + Send + 'static`
    #[cfg(feature = "compress-zstd")]
    Zstd(Box<ZstdDecoder<'static, Writer>>),
}

impl ContentDecoder {
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

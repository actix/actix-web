//! Stream decoders.

use std::{
    future::Future,
    io::{self, Write as _},
    pin::Pin,
    task::{Context, Poll},
};

use actix_rt::task::{spawn_blocking, JoinHandle};
use brotli2::write::BrotliDecoder;
use bytes::Bytes;
use flate2::write::{GzDecoder, ZlibDecoder};
use futures_core::{ready, Stream};

use crate::{
    encoding::Writer,
    error::{BlockingError, PayloadError},
    http::header::{ContentEncoding, HeaderMap, CONTENT_ENCODING},
};

const MAX_CHUNK_SIZE_DECODE_IN_PLACE: usize = 2049;

pub struct Decoder<S> {
    decoder: Option<ContentDecoder>,
    stream: S,
    eof: bool,
    fut: Option<JoinHandle<Result<(Option<Bytes>, ContentDecoder), io::Error>>>,
}

impl<S> Decoder<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>>,
{
    /// Construct a decoder.
    #[inline]
    pub fn new(stream: S, encoding: ContentEncoding) -> Decoder<S> {
        let decoder = match encoding {
            ContentEncoding::Br => Some(ContentDecoder::Br(Box::new(
                BrotliDecoder::new(Writer::new()),
            ))),
            ContentEncoding::Deflate => Some(ContentDecoder::Deflate(Box::new(
                ZlibDecoder::new(Writer::new()),
            ))),
            ContentEncoding::Gzip => Some(ContentDecoder::Gzip(Box::new(
                GzDecoder::new(Writer::new()),
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
            .map(ContentEncoding::from)
            .unwrap_or(ContentEncoding::Identity);

        Self::new(stream, encoding)
    }
}

impl<S> Stream for Decoder<S>
where
    S: Stream<Item = Result<Bytes, PayloadError>> + Unpin,
{
    type Item = Result<Bytes, PayloadError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(ref mut fut) = self.fut {
                let (chunk, decoder) =
                    ready!(Pin::new(fut).poll(cx)).map_err(|_| BlockingError)??;

                self.decoder = Some(decoder);
                self.fut.take();

                if let Some(chunk) = chunk {
                    return Poll::Ready(Some(Ok(chunk)));
                }
            }

            if self.eof {
                return Poll::Ready(None);
            }

            match ready!(Pin::new(&mut self.stream).poll_next(cx)) {
                Some(Err(err)) => return Poll::Ready(Some(Err(err))),

                Some(Ok(chunk)) => {
                    if let Some(mut decoder) = self.decoder.take() {
                        if chunk.len() < MAX_CHUNK_SIZE_DECODE_IN_PLACE {
                            let chunk = decoder.feed_data(chunk)?;
                            self.decoder = Some(decoder);

                            if let Some(chunk) = chunk {
                                return Poll::Ready(Some(Ok(chunk)));
                            }
                        } else {
                            self.fut = Some(spawn_blocking(move || {
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
                    self.eof = true;

                    return if let Some(mut decoder) = self.decoder.take() {
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
    Deflate(Box<ZlibDecoder<Writer>>),
    Gzip(Box<GzDecoder<Writer>>),
    Br(Box<BrotliDecoder<Writer>>),
}

impl ContentDecoder {
    fn feed_eof(&mut self) -> io::Result<Option<Bytes>> {
        match self {
            ContentDecoder::Br(ref mut decoder) => match decoder.flush() {
                Ok(()) => {
                    let b = decoder.get_mut().take();

                    if !b.is_empty() {
                        Ok(Some(b))
                    } else {
                        Ok(None)
                    }
                }
                Err(e) => Err(e),
            },

            ContentDecoder::Gzip(ref mut decoder) => match decoder.try_finish() {
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

            ContentDecoder::Deflate(ref mut decoder) => match decoder.try_finish() {
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
        }
    }

    fn feed_data(&mut self, data: Bytes) -> io::Result<Option<Bytes>> {
        match self {
            ContentDecoder::Br(ref mut decoder) => match decoder.write_all(&data) {
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
                Err(e) => Err(e),
            },

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
                Err(e) => Err(e),
            },
        }
    }
}

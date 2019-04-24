use std::io::{self, Write};

use actix_threadpool::{run, CpuFuture};
#[cfg(feature = "brotli")]
use brotli2::write::BrotliDecoder;
use bytes::Bytes;
#[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
use flate2::write::{GzDecoder, ZlibDecoder};
use futures::{try_ready, Async, Future, Poll, Stream};

use super::Writer;
use crate::error::PayloadError;
use crate::http::header::{ContentEncoding, HeaderMap, CONTENT_ENCODING};

const INPLACE: usize = 2049;

pub struct Decoder<S> {
    decoder: Option<ContentDecoder>,
    stream: S,
    eof: bool,
    fut: Option<CpuFuture<(Option<Bytes>, ContentDecoder), io::Error>>,
}

impl<S> Decoder<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    /// Construct a decoder.
    #[inline]
    pub fn new(stream: S, encoding: ContentEncoding) -> Decoder<S> {
        let decoder = match encoding {
            #[cfg(feature = "brotli")]
            ContentEncoding::Br => Some(ContentDecoder::Br(Box::new(
                BrotliDecoder::new(Writer::new()),
            ))),
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
            ContentEncoding::Deflate => Some(ContentDecoder::Deflate(Box::new(
                ZlibDecoder::new(Writer::new()),
            ))),
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
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
        let encoding = if let Some(enc) = headers.get(&CONTENT_ENCODING) {
            if let Ok(enc) = enc.to_str() {
                ContentEncoding::from(enc)
            } else {
                ContentEncoding::Identity
            }
        } else {
            ContentEncoding::Identity
        };

        Self::new(stream, encoding)
    }
}

impl<S> Stream for Decoder<S>
where
    S: Stream<Item = Bytes, Error = PayloadError>,
{
    type Item = Bytes;
    type Error = PayloadError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            if let Some(ref mut fut) = self.fut {
                let (chunk, decoder) = try_ready!(fut.poll());
                self.decoder = Some(decoder);
                self.fut.take();
                if let Some(chunk) = chunk {
                    return Ok(Async::Ready(Some(chunk)));
                }
            }

            if self.eof {
                return Ok(Async::Ready(None));
            }

            match self.stream.poll()? {
                Async::Ready(Some(chunk)) => {
                    if let Some(mut decoder) = self.decoder.take() {
                        if chunk.len() < INPLACE {
                            let chunk = decoder.feed_data(chunk)?;
                            self.decoder = Some(decoder);
                            if let Some(chunk) = chunk {
                                return Ok(Async::Ready(Some(chunk)));
                            }
                        } else {
                            self.fut = Some(run(move || {
                                let chunk = decoder.feed_data(chunk)?;
                                Ok((chunk, decoder))
                            }));
                        }
                        continue;
                    } else {
                        return Ok(Async::Ready(Some(chunk)));
                    }
                }
                Async::Ready(None) => {
                    self.eof = true;
                    return if let Some(mut decoder) = self.decoder.take() {
                        Ok(Async::Ready(decoder.feed_eof()?))
                    } else {
                        Ok(Async::Ready(None))
                    };
                }
                Async::NotReady => break,
            }
        }
        Ok(Async::NotReady)
    }
}

enum ContentDecoder {
    #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
    Deflate(Box<ZlibDecoder<Writer>>),
    #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
    Gzip(Box<GzDecoder<Writer>>),
    #[cfg(feature = "brotli")]
    Br(Box<BrotliDecoder<Writer>>),
}

impl ContentDecoder {
    #[allow(unreachable_patterns)]
    fn feed_eof(&mut self) -> io::Result<Option<Bytes>> {
        match self {
            #[cfg(feature = "brotli")]
            ContentDecoder::Br(ref mut decoder) => match decoder.finish() {
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
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
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
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
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
            _ => Ok(None),
        }
    }

    #[allow(unreachable_patterns)]
    fn feed_data(&mut self, data: Bytes) -> io::Result<Option<Bytes>> {
        match self {
            #[cfg(feature = "brotli")]
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
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
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
            #[cfg(any(feature = "flate2-zlib", feature = "flate2-rust"))]
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
            _ => Ok(Some(data)),
        }
    }
}
